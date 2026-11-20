<!-- Notes for the next release, in plain English. The release workflow puts this file at the top
of the draft release, above GitHub's list of merged PRs. After publishing, empty everything below
this comment for the release after. -->

## New

- **An optional AI pass for messy self-corrections.** Say "ship it today, sorry, ship it Friday after QA signs off" and get "ship it Friday after QA signs off." A local model ([Ollama](https://ollama.com) with `gemma3:4b`) runs only when "actually", "sorry", "I mean", "no wait", "scratch that" or "never mind" follows a comma or full stop. It may only remove words, never add them, and must keep how you ended. After 1.5 s Murmur pastes its usual text instead. Off by default: set `rewrite = true`. Nothing leaves your computer.
