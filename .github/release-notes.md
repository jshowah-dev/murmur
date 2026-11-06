<!-- Notes for the next release, in plain English. The release workflow puts this file at the top
of the draft release, above GitHub's list of merged PRs. After publishing, empty everything below
this comment for the release after. -->

## Fixed

- **Dictionary words no longer take over words that only sound like them.** A sound-alike match now has to be spelled close to the word, too. Before, an entry like SQL ("sequel") turned every "scroll" into "SQL".
