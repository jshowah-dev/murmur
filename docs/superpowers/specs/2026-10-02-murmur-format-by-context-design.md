# Murmur format by context: email

Date: 2026-10-02. Status: design approved in chat, pending review of this written spec.

## Goal

The same spoken sentence lands differently depending on the app it is pasted into. v1 has one context, **email**: a dictated greeting gets its own line and a dictated sign-off gets its own lines. Everywhere else the output is exactly what it is today.

You say: "hi Sarah thanks for the update I'll review it tomorrow and send notes by Friday thanks Jeff"

Today, everywhere:

```
Hi Sarah, thanks for the update. I'll review it tomorrow and send notes by Friday. Thanks, Jeff.
```

In an email app, after this change:

```
Hi Sarah,

Thanks for the update. I'll review it tomorrow and send notes by Friday.

Thanks,
Jeff
```

## Decisions (Jeff, 2026-10-02)

| Question | Decision |
|---|---|
| Contexts in v1 | **Email only.** No terminal, chat or code-editor profile. The app lookup is generic so one can be added later. |
| How email is recognised | Exe name for mail clients; browser exe plus a window-title fragment for webmail. No reading of the page or the focused field. |
| How text is shaped | Rules only. No language model. |
| Default | **On.** `format_by_context = false` in `config.toml` turns it off. |
| Sign-off with no name ("…by Friday. Thanks.") | Moves to its own paragraph; its punctuation is left alone. |
| Configuration | The app and title lists are in `config.toml` with built-in defaults. No new UI. |

Out of scope: any other context, telling a compose box from a search bar, tone changes, paragraph breaks from pauses, auto-inserting a signature, a settings window, the version bump and release.

## Components

| Unit | File | Job |
|---|---|---|
| Context detection | `src/context.rs` (new, lib) | `enum Profile { Plain, Email }`. `classify(exe: &str, title: &str, cfg: &Config) -> Profile` is pure and unit-tested. `detect(cfg: &Config) -> Profile` reads the foreground window's exe name and title through Win32 and calls `classify`; any Win32 failure returns `Plain`. |
| Email formatting | `src/email.rs` (new, lib) | `format(text: &str) -> String`: the greeting and sign-off rules below. Pure, no Win32, unit-tested. |
| Cleanup | `src/cleanup.rs` | `clean` gains a `profile: Profile` parameter. After `tidy` and the space trim, before `snippets::restore`, it calls `email::format` when the profile is `Email`. |
| Pipeline | `src/pipeline.rs` | `stop()` calls `context::detect(&self.cfg)` just before `cleanup::clean` and passes the result. When `format_by_context` is false it passes `Plain` without calling `detect`. Logs `profile: email` or `profile: plain` at INFO. |
| Config | `src/config.rs` | Three new fields, below. |
| Docs | `README.md` | The three settings in the config table; a short "Email formatting" section with the example above and the known limits. |

`Win32_System_Threading` is already enabled in `Cargo.toml`; no new dependency.

## Behaviour

### Recognising email

`detect` takes `GetForegroundWindow`, then:

- **exe**: `GetWindowThreadProcessId` → `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` → `QueryFullProcessImageNameW`, file name only.
- **title**: `GetWindowTextW`.

`classify` returns `Email` when either holds, compared without regard to case:

1. `exe` is in `email_apps`.
2. `exe` is a browser **and** `title` contains one of `email_titles`.

Browsers are a fixed list in code: `chrome.exe`, `msedge.exe`, `firefox.exe`, `brave.exe`, `opera.exe`, `vivaldi.exe`, `arc.exe`. A title fragment never counts outside a browser, so a Word file named "Outlook migration.docx" stays `Plain`.

The window is read when the key is released, at the same moment the text is cleaned and pasted, so the profile always belongs to the window that receives the paste.

### Config

| Key | Default | Meaning |
|---|---|---|
| `format_by_context` | `true` | Shape text for the app it lands in. `false` gives today's output everywhere. |
| `email_apps` | `["OUTLOOK.EXE", "olk.exe", "thunderbird.exe"]` | Exe names treated as email. |
| `email_titles` | `["Gmail", "Outlook", "Proton Mail", "Yahoo Mail"]` | Window-title fragments treated as email, in browsers only. |

`Config` is `#[serde(default)]`, so an existing `config.toml` without these keys gets the defaults. As with every other setting, a change takes effect on the next start.

### Greeting rule

Looks only at the **start** of the dictation.

- The first words are a greeting opener: `hi`, `hello`, `hey`, `dear`, `good morning`, `good afternoon`, `good evening`, `greetings`.
- A comma directly on the opener ("Hi, Sarah.") is dropped.
- After the opener come **0 to 3** name words, and the last of them (or the opener itself when there are none) ends in `,` `.` or `!`, or is the last word of the line.
- A name word is a capitalised word, or one of `there`, `all`, `everyone`, `everybody`, `team`, `folks`, `both`, `and`. `I` and its contractions (`I'm`, `I'll`, `I've`, `I'd`) are never name words. The full stop of `Mr.` `Mrs.` `Ms.` `Dr.` `Prof.` is part of the word and does not end the greeting.
- When that holds: that final mark becomes `,`, a blank line follows, and the next word is capitalised.
- When the words after the opener are not name words but the opener itself carries a comma ("Hi, just checking in."), the greeting is the opener alone.
- Otherwise the rule does not fire and the text is unchanged.

| In | Out |
|---|---|
| `Hi Sarah, thanks for the update.` | `Hi Sarah,` ⏎⏎ `Thanks for the update.` |
| `Hi, Sarah. Thanks for the update.` | `Hi Sarah,` ⏎⏎ `Thanks for the update.` |
| `Good morning, team. The build is ready.` | `Good morning team,` ⏎⏎ `The build is ready.` |
| `Hello. Quick question.` | `Hello,` ⏎⏎ `Quick question.` |
| `Hi, just checking in. Are you free?` | `Hi,` ⏎⏎ `Just checking in. Are you free?` |
| `Dear Mr. Smith, the report is attached.` | `Dear Mr. Smith,` ⏎⏎ `The report is attached.` |
| `Hey Jeff can you send the file` | unchanged (no punctuation ends the greeting) |
| `Hi, I'm Jeff. I'm writing about the invoice.` | `Hi,` ⏎⏎ `I'm Jeff. I'm writing about the invoice.` |
| `Hi Sarah and Tom and Priya and Dev, the build is ready.` | unchanged (more than 3 words) |
| `The build is ready. Hi Sarah, thanks.` | unchanged (not at the start) |

A dictation that is only a greeting ("Hi Sarah.") becomes `Hi Sarah,` followed by a blank line, so the next dictation starts the body.

### Sign-off rule

Looks only at the **end** of the dictation.

- The closer is one of: `thanks`, `thank you`, `thanks again`, `many thanks`, `thanks so much`, `best`, `best regards`, `kind regards`, `warm regards`, `regards`, `cheers`, `sincerely`, `talk soon`. The longest match wins. A phrase never spans punctuation: in "…works best. Regards, Jeff." the closer is `regards`, not `best regards`. The same holds for greeting openers.
- The closer starts a sentence: it is the first word of the dictation or the word before it ends in `.` `!` or `?`.
- After the closer come **0 to 3** name words and then the end of the text. A name word is a capitalised word; only the last may carry punctuation.
- **With a name** (1 to 3 words): a blank line, the closer followed by `,`, a line break, then the name with its trailing `.` `!` or `,` removed.
- **With no name**: a blank line, then the closer exactly as dictated.
- More than 3 words after the closer, words after it that are not name words, or a closer in mid-sentence: the rule does not fire.
- A dictation that is only a closer with no name ("Thanks.") is unchanged.

| In | Out |
|---|---|
| `…notes by Friday. Thanks, Jeff.` | `…notes by Friday.` ⏎⏎ `Thanks,` ⏎ `Jeff` |
| `…notes by Friday. Best regards. Jeff Showah.` | `…notes by Friday.` ⏎⏎ `Best regards,` ⏎ `Jeff Showah` |
| `…notes by Friday. Thanks.` | `…notes by Friday.` ⏎⏎ `Thanks.` |
| `Thanks, Jeff.` (whole dictation) | `Thanks,` ⏎ `Jeff` |
| `Thanks for the update, I'll look tomorrow.` | unchanged (more than 3 words follow) |
| `I said thanks, Jeff.` | unchanged (closer is mid-sentence) |
| `I think Tuesday works best. Regards, Jeff.` | `I think Tuesday works best.` ⏎⏎ `Regards,` ⏎ `Jeff` |
| `…notes by Friday. Thanks a lot.` | unchanged ("a lot" is not a name) |
| `…notes by Friday. Thanks, I appreciate it.` | unchanged (not a name) |

### Order and interactions

- Order inside `clean`: fillers → snippet marking → dictionary → spoken commands → `tidy` → **email format** → snippet restore. Snippet text is still pasted verbatim, and a snippet at the end of a dictation is never taken for a name.
- Both rules can fire on one dictation. When the greeting and the sign-off would overlap (a dictation that is only "Hi Sarah."), the greeting wins and the sign-off is not tried on the same words.
- The middle of a dictation is never touched, so an email dictated in several goes works: only the first chunk can get a greeting break and only the last a sign-off.
- A line break already produced by "new line" / "new paragraph" at the spot where a rule would add one is not doubled.
- History, the correction window and "copy last" all carry the formatted text, since it is the `cleaned` text. A correction is re-pasted as edited; it is not formatted again.

### Privacy

The window title can hold an email subject or address. It is used for the match and then dropped: it is **never logged**, at any level. The log line is the profile name only.

## Known limits (accepted for v1)

- Webmail's search bar and any other field in the tab get email formatting too; the title cannot tell them apart.
- "Thanks, Sarah." at the end is formatted as a sign-off even when Sarah was being thanked.
- Any one to three capitalised words after a closing sentence are read as the name: "…Friday. Thanks. Bye." becomes `Thanks,` ⏎ `Bye`.
- When the recogniser puts no punctuation after the greeting name, the greeting rule does not fire and the output is today's.
- A browser tab whose title happens to contain a fragment (an article titled "Gmail tips") is treated as email.
- An elevated mail client, whose exe name cannot be read from a non-elevated Murmur, is treated as `Plain`.

## Testing

Unit tests, no UI or Win32:

- `context::classify`: each default mail exe in mixed case; a browser with and without a mail title; a mail title in a non-browser; empty exe and title; a user-added exe and fragment; lists emptied in config.
- `email::format`: every row of the two tables above; both rules on one dictation (the Goal example); greeting-only and sign-off-only dictations; text with existing line breaks; text holding snippet placeholders at the start and end; empty text.
- `cleanup::clean`: `Plain` output is byte-for-byte what it is today for the existing test cases; `Email` runs the Goal example from raw text to final text.
- `config`: defaults for the three keys; an old `config.toml` without them loads.

Smoke test (Jeff, on the built app): the Goal example dictated into classic Outlook, into Gmail in a browser, and into Notepad (must be unchanged); then `format_by_context = false` and Outlook again (must be unchanged). Check `murmur.log` shows `profile: email` / `profile: plain` and no window title.
