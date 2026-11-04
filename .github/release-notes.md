<!-- Notes for the next release, in plain English. The release workflow puts this file at the top
of the draft release, above GitHub's list of merged PRs. After publishing, empty everything below
this comment for the release after. -->

## Fixed

- **Dictations paste your words, not the old clipboard.** On some Macs since macOS 27.2, a dictation sometimes pasted whatever you had copied before instead of what you said. Murmur put your clipboard back too soon after pasting, before slower apps had read it. It now waits a little longer, and leaves the clipboard alone if you copy something else in the meantime. Windows gets the same change.
