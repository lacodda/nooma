# Laptop won't wake up from sleep mode

If your laptop screen stays black after you open the lid or press a key, but the power light is on and fans might even spin briefly, it's usually stuck between sleep and fully awake rather than actually off — a fairly common issue after a Windows or driver update.

## Quick things to try first

Hold the power button for ten seconds to force a hard shutdown, then power back on normally — this clears a stuck state without losing unsaved work in most modern laptops, since sleep mode keeps everything in RAM until it either resumes or the battery runs critically low. If that works but recurs often, check for pending graphics driver updates, since outdated GPU drivers are a frequent cause of failed wake-ups.

## If it keeps happening

Open power settings and check what triggers sleep and what's allowed to wake the machine — a misbehaving USB device or network adapter can sometimes prevent a clean wake cycle. Disabling "fast startup" (on Windows) or resetting the power management settings for network adapters in Device Manager often resolves recurring wake failures.
