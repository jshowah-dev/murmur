<!-- Notes for the next release, in plain English. The release workflow puts this file at the top
of the draft release, above GitHub's list of merged PRs. After publishing, empty everything below
this comment for the release after. -->

## What's new

- **Less waiting after long dictations.** Murmur now transcribes at natural pauses while you're still talking, so letting go of the key only leaves the last stretch to do. The pill also switches to its working look the moment you let go.
- **Stutters are dropped.** "the the" and cut-off starts like "pro professional" come out once. Words people double on purpose ("had had", "very very") stay. Turn it off with `drop_stutters = false`.
- **Questions get a question mark.** A sentence shaped like a question ("Can you send it", "What do you think", "…, right") ends in "?" instead of ".".
- **Spoken symbols.** "jeff at sign showah dot co" becomes "jeff@showah.co", and "slash users slash jeff" becomes "/users/jeff". Say "literal dot" to keep the word. The words live in a `[symbols]` table in `config.toml`.
- **"Scratch that."** Said on its own, it undoes your last dictation if it's under a minute old and in the window in front. Said mid-dictation, it drops what you said before it. Starting with "Scratch that." and going on replaces the last dictation.
- **Self-corrections.** "Tuesday, no, Wednesday" becomes "Wednesday", and "at 3, I mean 4" becomes "at 4". Turn it off with `self_corrections = false`.
- **Windows: the pill moves with your voice.** It now fills about as much as it does on a Mac.
- **Windows: fewer audio dropouts.** Murmur keeps a bigger microphone buffer, so a busy moment no longer drops bits of speech.
