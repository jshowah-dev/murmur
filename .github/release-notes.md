<!-- Notes for the next release, in plain English. The release workflow puts this file at the top
of the draft release, above GitHub's list of merged PRs. After publishing, empty everything below
this comment for the release after. -->

## Fixed

- **Spoken dates and times come out as digits.** When the speech model writes a month and day or an hour and minute as words, Murmur now writes it as numbers, and the words around it decide which: "from the eleven twenty-two date" → "from the 11/22 date", "due eleven twenty-second" → "due 11/22", "meet at three thirty" → "meet at 3:30", "two thirty pm" → "2:30 pm". With no hint, a real date wins: "eleven twenty-two works" → "11/22 works". Say "at", "pm" or "o'clock" to get a time.
