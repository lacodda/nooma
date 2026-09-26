//! Fetching an embedding model: the one part of nooma that opens a network
//! connection.
//!
//! Everything else in nooma reads files. This crate exists so that claim can
//! be checked rather than trusted — `nooma-core`, which every consumer links,
//! has no HTTP client and no TLS among its dependencies at all, and a test
//! holds it to that. What reaches the network is here, and only here: the
//! files of a model from the catalogue, at the commit it was pinned at, and
//! nothing about the person's files, ever.
//!
//! A file is written to `<name>.part` beside its place, checked against the
//! size and SHA-256 it was pinned with, and only then renamed into place — so
//! a model directory never holds a file that is not the one measured. A
//! `.part` left by an interrupted fetch is resumed from where it stopped.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use nooma_core::model::{ModelFile, ModelSpec};
use sha2::{Digest, Sha256};

/// Where models come from.
pub const HUB: &str = "https://huggingface.co";

/// How far a fetch has got, for a progress bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchProgress<'a> {
    /// The file being fetched or checked.
    pub file: &'a str,
    /// Bytes of it on disk so far.
    pub done: u64,
    /// Its size.
    pub total: u64,
}

/// What a fetch did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fetched {
    /// Files downloaded, in whole or in part.
    pub downloaded: Vec<&'static str>,
    /// Files that were already there and checked out.
    pub verified: Vec<&'static str>,
    /// Bytes received over the network.
    pub received: u64,
}

/// What can go wrong fetching a model.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    /// The request did not get a response.
    #[error("{url}: {reason}")]
    Http {
        /// What was asked for.
        url: String,
        /// What went wrong.
        reason: String,
    },

    /// The server answered with something other than the file.
    #[error("{url}: the server answered {status}")]
    Status {
        /// What was asked for.
        url: String,
        /// The HTTP status.
        status: u16,
    },

    /// The file arrived, and is not the one that was pinned. It is deleted.
    #[error(
        "{file} arrived as {found_bytes} bytes hashing to {found}, not the {bytes} bytes hashing to {expected} it was pinned at; it was deleted — fetch again, and if it happens again the file upstream has changed"
    )]
    Mismatch {
        /// The file.
        file: String,
        /// The pinned hash.
        expected: String,
        /// The hash of what arrived.
        found: String,
        /// The pinned size.
        bytes: u64,
        /// The size of what arrived.
        found_bytes: u64,
    },

    /// The disk refused.
    #[error("{path}: {source}")]
    Io {
        /// The path.
        path: PathBuf,
        /// The failure.
        source: std::io::Error,
    },
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> FetchError + '_ {
    move |source| FetchError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Fetch a model from the Hub into a models directory.
///
/// Files already there are checked against their pinned hash, and kept when
/// they match; a file that does not match is fetched again.
pub fn fetch(spec: &ModelSpec, models: &Path, progress: &mut dyn FnMut(FetchProgress<'_>)) -> Result<Fetched, FetchError> {
    Fetcher::hub().fetch(spec, models, progress)
}

/// How a model is fetched: from where, and how patiently.
#[derive(Debug, Clone)]
pub struct Fetcher {
    base: String,
    window: Duration,
    attempts: u32,
}

impl Fetcher {
    /// From the Hugging Face Hub.
    pub fn hub() -> Self {
        Self::new(HUB)
    }

    /// From another server laid out like the Hub.
    pub fn new(base: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            window: Duration::from_secs(60),
            attempts: 5,
        }
    }

    /// The longest one request may keep sending a body before the fetch
    /// hangs up and asks again for the rest.
    ///
    /// A connection can go on delivering a trickle - kilobytes a second from
    /// a cold edge of a CDN, while a new connection gets megabytes - and the
    /// HTTP client has no way to call that stalled. Asking afresh every
    /// window caps what a bad connection can cost, and on a good one costs a
    /// new request per few hundred megabytes.
    pub fn window(mut self, window: Duration) -> Self {
        self.window = window;
        self
    }

    /// Fetch a model into a models directory. See [`fetch`].
    pub fn fetch(&self, spec: &ModelSpec, models: &Path, progress: &mut dyn FnMut(FetchProgress<'_>)) -> Result<Fetched, FetchError> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .user_agent(format!("nooma/{}", env!("CARGO_PKG_VERSION")))
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .timeout_recv_body(Some(self.window))
            // Statuses are read below: a 416 on a resume means something
            // other than failure.
            .http_status_as_error(false)
            .build()
            .into();

        let mut fetched = Fetched::default();
        for file in spec.files {
            let dest = spec.file_path(models, file);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(io(parent))?;
            }
            if dest.metadata().is_ok_and(|meta| meta.len() == file.bytes) {
                let found = hash_file(&dest, file, progress)?;
                if found == file.sha256 {
                    fetched.verified.push(file.path);
                    continue;
                }
            }
            let url = format!("{}/{}/resolve/{}/{}", self.base, spec.repository, spec.revision, file.path);
            fetched.received += self.download(&agent, &url, &dest, file, progress)?;
            fetched.downloaded.push(file.path);
        }
        Ok(fetched)
    }

    /// Download one file into place through its `.part`; returns the bytes
    /// received.
    ///
    /// Asks again for the rest whenever a request ends early - a dropped
    /// connection, or the window running out - and gives up only after
    /// several requests in a row brought nothing.
    fn download(&self, agent: &ureq::Agent, url: &str, dest: &Path, file: &ModelFile, progress: &mut dyn FnMut(FetchProgress<'_>)) -> Result<u64, FetchError> {
        let part = part_path(dest);
        let mut hasher = Sha256::new();

        // What a previous fetch left: hashed again, since the hash of the
        // whole file is what gets checked and the process that wrote it is
        // gone.
        let mut have = match part.metadata() {
            Ok(meta) if meta.len() < file.bytes => meta.len(),
            Ok(_) => {
                std::fs::remove_file(&part).map_err(io(&part))?;
                0
            }
            Err(_) => 0,
        };
        if have > 0 {
            let mut input = File::open(&part).map_err(io(&part))?;
            let mut buffer = vec![0u8; 1 << 20];
            loop {
                let read = input.read(&mut buffer).map_err(io(&part))?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
            }
        }

        let mut received = 0u64;
        let mut fruitless = 0u32;
        let mut last_error = String::new();
        while have < file.bytes {
            if fruitless >= self.attempts {
                return Err(FetchError::Http {
                    url: url.to_string(),
                    reason: format!("{} requests in a row brought nothing; the last said: {last_error}", self.attempts),
                });
            }
            let mut request = agent.get(url);
            if have > 0 {
                request = request.header("Range", &format!("bytes={have}-"));
            }
            let response = match request.call() {
                Ok(response) => response,
                Err(error) => {
                    last_error = error.to_string();
                    fruitless += 1;
                    continue;
                }
            };
            let append = match response.status().as_u16() {
                206 if have > 0 => true,
                // The whole file, instead of the rest of it.
                200 => {
                    have = 0;
                    hasher = Sha256::new();
                    false
                }
                status => {
                    // A range past the end: what is on disk cannot be
                    // resumed, so the next fetch starts from nothing.
                    if status == 416 {
                        let _ = std::fs::remove_file(&part);
                    }
                    return Err(FetchError::Status { url: url.to_string(), status });
                }
            };

            let mut output = File::options()
                .create(true)
                .write(true)
                .append(append)
                .truncate(!append)
                .open(&part)
                .map_err(io(&part))?;
            // Never much more than the file was pinned at: a server sending
            // more is not sending this file. One byte over, because the limit
            // refuses the read that would find the end of a body exactly its
            // size - and a byte too many then fails the check below.
            let mut body = response.into_body().into_with_config().limit(file.bytes - have + 1).reader();
            let mut buffer = vec![0u8; 1 << 16];
            let before = have;
            loop {
                match body.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        output.write_all(&buffer[..read]).map_err(io(&part))?;
                        hasher.update(&buffer[..read]);
                        have += read as u64;
                        received += read as u64;
                        progress(FetchProgress {
                            file: file.path,
                            done: have,
                            total: file.bytes,
                        });
                    }
                    // The window ran out, or the connection dropped. What
                    // arrived is on disk; the next request asks for the rest.
                    Err(error) => {
                        last_error = error.to_string();
                        break;
                    }
                }
            }
            output.flush().map_err(io(&part))?;
            if have > file.bytes {
                break;
            }
            fruitless = if have > before { 0 } else { fruitless + 1 };
        }

        let found = hex(&hasher.finalize());
        if have != file.bytes || found != file.sha256 {
            let _ = std::fs::remove_file(&part);
            return Err(FetchError::Mismatch {
                file: file.path.to_string(),
                expected: file.sha256.to_string(),
                found,
                bytes: file.bytes,
                found_bytes: have,
            });
        }
        std::fs::rename(&part, dest).map_err(io(dest))?;
        Ok(received)
    }
}

/// Hash a file on disk, reporting progress as it is read.
fn hash_file(path: &Path, file: &ModelFile, progress: &mut dyn FnMut(FetchProgress<'_>)) -> Result<String, FetchError> {
    let mut hasher = Sha256::new();
    let mut input = File::open(path).map_err(io(path))?;
    let mut buffer = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        let read = input.read(&mut buffer).map_err(io(path))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        done += read as u64;
        progress(FetchProgress {
            file: file.path,
            done,
            total: file.bytes,
        });
    }
    Ok(hex(&hasher.finalize()))
}

fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    dest.with_file_name(name)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
