<!-- Notes for the next release, in plain English. The release workflow puts this file at the top
of the draft release, above GitHub's list of merged PRs. After publishing, empty everything below
this comment for the release after. -->

## What's new

- **"5 point 2" types "5.2".** "point" between two numbers is now a decimal point. Anywhere else it stays a word.
- **Windows: updates work on networks that inspect secure connections.** Many work networks re-sign secure connections with their own certificate, so the update check and downloads failed there. Murmur now checks certificates against Windows' own certificate store, as browsers do.
- **Windows: the installer no longer stalls when Murmur is running.** Murmur now closes as soon as setup asks, so installing over a running copy doesn't sit idle for up to a minute.
- **Crashes leave a note in the log.** If Murmur stops unexpectedly, `murmur.log` now records what went wrong, so it can be fixed.
