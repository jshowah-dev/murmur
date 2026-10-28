<!-- Notes for the next release, in plain English. The release workflow puts this file at the top
of the draft release, above GitHub's list of merged PRs. After publishing, empty everything below
this comment for the release after. -->

**Live preview.** Your words now appear just below the cursor while you speak, so you can see they're being heard before you let go. A small speech model (about 110 MB) downloads in the background the first time Murmur starts; the preview begins with the first dictation after that. What's pasted is unchanged: the main speech model still writes the final text. Turn it off with `live_preview = false` in `config.toml`.
