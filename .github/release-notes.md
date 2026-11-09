<!-- Notes for the next release, in plain English. The release workflow puts this file at the top
of the draft release, above GitHub's list of merged PRs. After publishing, empty everything below
this comment for the release after. -->

## Fixed

- **Your clipboard comes back whole.** Murmur borrows the clipboard to paste, then puts back what was there. It used to put back only text, so a copied image or file was lost. Now everything comes back, including when you quit right after a dictation.
- **Short words at the start aren't dropped.** A quick "Yes" or "Okay" said as you pressed the key could come back as "Didn't catch that".
- **Accented words no longer stop a dictation.** A word ending in a letter like "é" or "ë" ("café", "Zoë") could make Murmur fail and lose what you said.
- **A restarted word is cleaned up.** "I'm guess guessing" now comes out as "I'm guessing".
- **Real words aren't taken for "um".** "The ER" and "5 mm" now keep their words.
- **A hands-free recording can't get stuck.** Opening History or About from the tray while recording could leave the mic on.
- **Quitting keeps a dictation in progress.** Quitting, or an update installing, now waits for a dictation you've already let go of.
- **A failed paste says so.** On a Mac without the Accessibility permission, Murmur now tells you instead of doing nothing.
- **About's links open on a Mac.**
