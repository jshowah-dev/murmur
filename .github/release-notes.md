
## Fixed

- **"Claude" or "cloud", picked by what's around it.** The speech model mixes the two up, and a dictionary entry could only force one of them every time. Murmur now reads the neighbouring words: "ask Claude", "Claude Code", "Claude said", "the Claude app" → Claude; "in the cloud", "cloud storage", "private cloud" → cloud. With no clue either way, the model's word stays. Fix-last no longer learns a swap between the two.
