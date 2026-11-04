<!-- Notes for the next release, in plain English. The release workflow puts this file at the top
of the draft release, above GitHub's list of merged PRs. After publishing, empty everything below
this comment for the release after. -->

## Fixed

- **You're told when an update is available, even after looking in About.** Opening About when a new version was out added it to the tray menu, but then Murmur never showed the "update available" notification for that version. It now shows it at the next hourly check.
