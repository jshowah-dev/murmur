<!-- Notes for the next release, in plain English. The release workflow puts this file at the top
of the draft release, above GitHub's list of merged PRs. After publishing, empty everything below
this comment for the release after. -->

## Changed

- **No more boxes at your cursor.** Murmur's messages ("Didn't catch that", "Your mic is silent", an update) now come as notifications instead of a bubble at your cursor or under the notch. The dot still flies to your cursor when you let go. To bring the bubbles back, set `message_bubbles = true` in config.toml.
- **The live preview is off by default.** Your words no longer show under the cursor while you speak, and its extra model isn't downloaded. To turn it back on, set `live_preview = true` in config.toml.
