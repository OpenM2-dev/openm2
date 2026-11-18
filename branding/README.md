# openm2's logo

The project's name and logo, as its own releases use them.

| File | What it is | Where it's used |
| --- | --- | --- |
| `openm2-banner.png` | The wordmark on `#060709`, 1280 × 320 | The top of this repo's README; the launcher's window (`deploy/windows/launcher.toml` and `window.toml`, drawn at 640 × 160) |
| `openm2.ico` | "M2" from the wordmark, 16 to 256 px | The setup, its shortcuts and its entry in Windows' Apps |
| `setup-wizard.png` | "M2" over an ember glow, 430 × 824 | The setup wizard's tall picture |
| `setup-small.png` | "M2", 124 × 124 | The setup wizard's corner picture |

The window's colours go with them: the wordmark's gold, `#dfae4f`, for the accent, and `#060709` for the background.

## Terms

- **These files are not under the code's licence.** MIT and Apache-2.0 cover the code and the docs in this repo, not this folder. Apache-2.0 grants no rights to the project's name or marks either (its section 6).
- **You may show the logo unchanged** to point at openm2, for example in an article, a link or a list of projects.
- **Builds, forks and other projects that you ship use their own name and logo,** not these. Remove this folder and its uses (the setup's icon and pictures, the window's `[window]` and `window.toml`) from what you build.

## Changing them

The files are made outside this repo, from the wordmark's master. A change replaces them whole; never edit them here. `just no-assets` lets `.png` files in this folder through and refuses pictures everywhere else.
