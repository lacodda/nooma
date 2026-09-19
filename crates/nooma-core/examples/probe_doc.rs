//! Probe: walking commit history with gix, and what a commit yields.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let repo = gix::discover(&path)?;
    let head = repo.head_commit()?;
    let mut count = 0;
    for info in head.ancestors().all()? {
        let info = info?;
        let commit = repo.find_commit(info.id)?;
        let message = commit.message()?;
        let time = commit.time()?;
        let author = commit.author()?;
        if count < 3 {
            println!("--- {}", info.id.to_hex());
            println!("  summary: {:?}", message.summary().to_string());
            println!("  body: {:?}", message.body().map(|b| b.to_string()));
            println!("  author: {:?} <{}>", author.name.to_string(), author.email);
            println!("  seconds: {} offset {}", time.seconds, time.offset);
        }
        count += 1;
        if count > 200 {
            break;
        }
    }
    println!("total walked: {count}");
    Ok(())
}
