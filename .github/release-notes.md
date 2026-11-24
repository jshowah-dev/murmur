<!-- Notes for the next release, in plain English. The release workflow puts this file at the top
of the draft release, above GitHub's list of merged PRs. After publishing, empty everything below
this comment for the release after. -->

## Fixed

- **Numbers come out as digits.** When the speech model writes a number as words, or half and half, Murmur now writes it as digits: "Q 3 twenty twenty 7" → "Q3 2027", "twenty twenty-eight" → "2028", "two hundred and fifty" → "250", "twenty-first" → "21st", "five point two five percent" → "5.25%". A single word under ten in a sentence stays a word ("one thing", "the second time") unless your dictionary changes it.
- **No more guessed dates.** A pair of numbers with no hint is no longer turned into a date: "give me one second" stays as said. Say "on", "due" or "the … date" for a date, and "at" or "pm" for a time.
