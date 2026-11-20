<!-- Notes for the next release, in plain English. The release workflow puts this file at the top
of the draft release, above GitHub's list of merged PRs. After publishing, empty everything below
this comment for the release after. -->

## Fixed

- **The AI correction pass is ready when you need it.** Ollama unloads its model after five idle minutes, and reloading it took longer than Murmur waits, so the first correction after a break was pasted as said. Murmur now loads the model while you speak.
- **Names from the screen on the first dictation into an app.** An app can answer its first Accessibility question with nothing while it gets ready (Outlook did), so the first dictation found no names. Murmur now asks again at release when the first read came back empty.
