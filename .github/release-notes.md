<!-- Notes for the next release, in plain English. The release workflow puts this file at the top
of the draft release, above GitHub's list of merged PRs. After publishing, empty everything below
this comment for the release after. -->

## Fixed

- **Dictations paste your words, not the old clipboard, in busy apps too.** 0.5.7 still sometimes pasted what you had copied before, mostly on the first dictation after Murmur started, in apps that were slow to respond (the Claude app was one). Murmur now waits two seconds before putting your clipboard back, and still leaves it alone if you copy something else in that time.
