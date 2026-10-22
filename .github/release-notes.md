<!-- Notes for the next release, in plain English. The release workflow puts this file at the top
of the draft release, above GitHub's list of merged PRs. After publishing, empty everything below
this comment for the release after. -->

## What's new

- **The Mac version updates itself.** When a new version is out, click **Update** in the menu bar menu or the notification: Murmur downloads it, checks it, swaps it in and restarts. The Mac version is now signed and notarized by Apple: no more **Open Anyway** step, and updates keep their **Accessibility** permission. Updating from 0.5.0 asks for it once more, since that build was signed differently.
- **Back-to-back dictations no longer run together.** A dictation that lands right after another now starts with a space, so "now." and "Where's" no longer paste as "now.Where's".
- **Faster dictation on processors with fewer than 8 cores.** Murmur now runs one recognition thread per core, up to 8, instead of always 8. On a 4-core processor that roughly halves the wait. A `threads` value you set in `config.toml` still wins, except 8, the old default, which now follows your processor.
- **Help in the menu.** It opens the README, which now has a Troubleshooting section and system requirements.
