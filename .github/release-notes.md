<!-- Notes for the next release, in plain English. The release workflow puts this file at the top
of the draft release, above GitHub's list of merged PRs. After publishing, empty everything below
this comment for the release after. -->

## Fixed

- **One-word answers come through.** A quick "no", "yes" or "done" could be dropped as "Didn't catch that". Murmur now keeps speech as short as a tenth of a second.
- **The last word isn't clipped.** Murmur keeps listening for a tenth of a second after you let go of the key, so a word you finish as you release it is heard in full.

## Faster

- The app and caret lookups now run while your speech is being transcribed, not after it, and the Mac pastes 25 ms sooner.
