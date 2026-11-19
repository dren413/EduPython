# EduPython

<p align="center">
  <img src="./edupython_icon.png" alt="EduPython logo" width="190" />
</p>

<p align="center">
  A native Python editor for school: write code, run it, and see your classes as a <b>UML class diagram</b>.
</p>

<p align="center">
  <a href="https://www.edupython.lu">www.edupython.lu</a>
  ·
  <a href="https://github.com/dren413/EduPython/releases">Downloads</a>
  ·
  <a href="https://github.com/dren413/EduPython/releases/latest">Latest Release</a>
</p>

---

EduPython was created from a teacher's perspective to fill a gap in teaching tools.
Version 2 is a native rewrite in Rust. It drops the tkinter GUI designer and adds a live, two-way
UML class diagram. There is no web view, no Node.js, and nothing to install for pupils.

| Class | What they use |
|-------|---------------|
| **2CB** | Code only: editor + console (with `input()` support). Hide the UML panel. |
| **1CB** | Classes and pygame, with a live, two-way UML diagram. |

## Features

- **Editor**: Python highlighting, line numbers, auto-indent after `:`, Tab/Shift+Tab
  (also on several lines), Backspace removes a whole indent level, Ctrl+/ comments lines,
  undo/redo, adjustable font size (good for projectors).
- **Run** (F5) the main file or **run the open file** (Ctrl+F5). **Stop** with Shift+F5.
  Files are saved automatically before running.
- **Console**: output and errors, an **input field for `input()`**, clickable traceback lines.
  The line with the error is shown in red in the editor.
- **UML class diagram (left)**, built live from the code as you type:
  - boxes with three parts: class name / attributes `x: int` / methods `move(p:(int,int))`
  - types come from annotations or are guessed from values (`self.speed = (4, 3)` → `(int,int)`)
  - class attributes are underlined; `extends Sprite` is shown for external base classes
  - arrows for inheritance (between project classes) and associations (`self.balls: list[Ball]`)
  - drag boxes, scroll/pinch to zoom, double-click to open the code, **Export SVG** for worksheets
- **UML → code**: right-click in the diagram to add a **class** (optionally in its own file, with
  the import added to `main.py`), **attributes** (optionally as constructor parameters),
  **methods** with typed parameters and return types, or a **constructor**, or to **delete a method**.
  You can type types in UML style (`(int,int)`); the code gets `tuple[int, int]`.
- **File tabs** above the editor: one tab per file (tabs cannot be closed). **+** adds a file;
  right-click a tab to rename or delete it, or to make it the main file (green triangle).
- **Templates**: Script (2CB), Classes (1CB), Pygame (1CB, the Pong exercise 5.19), Empty.
- **Python**: uses the built-in Python (release builds) or finds an installed one.
  Click the Python version in the toolbar to see the interpreter and to install pygame into it.
- **Booklet**: the exam reference booklet (PDF) in a side panel, with clickable table of
  contents and links, page jump, zoom
  (buttons, Ctrl + mouse wheel or trackpad pinch) and **search** (Ctrl+F; finds French words
  without typing accents, and code such as `my_list[:]`). Text can be **selected and copied**
  (drag, double-click a word, Ctrl+C or right-click > Copy) and pastes into code with its
  indentation and plain `-`, `*` and `'` characters.
  It is installed by the school next to the program (see below), so a new booklet
  needs no new EduPython version.
- The editor scrolls only vertically; long lines wrap. Ctrl + mouse wheel or trackpad pinch
  changes the code size (also A− / A+ in the toolbar).
- On a Mac, Ctrl works like ⌘ for all shortcuts (copy, paste, undo, save, …).
- Light/dark mode and interface zoom (Ctrl +/−).

## School setup (IT): booklet and fixed Python

Put these files **in the same folder as `EduPython.exe`** (e.g. `C:\Program Files\EduPython\`):

```
EduPython.exe
edupython.ini          <- copy of packaging/edupython.example.ini
Python reference.pdf    <- the booklet named in the ini
```

`edupython.ini` sets:

- `[python] path = ...` (repeatable): the interpreters to use, **always tried first**.
  If none work and `fallback_search = yes`, EduPython searches the computer as usual.
- `allow_change = no`: pupils cannot pick another interpreter (the default with an ini).
- `allow_install = no`: hides *Install pygame* (the default with an ini).
- `packages = pygame, numpy`: checked at start; missing ones turn the toolbar dot orange.
- `packages_path = ...`: extra package folders (added to `PYTHONPATH`).
- `[booklet] file = ...` and `title = ...`: the booklet PDF (relative to the ini).
- `[display] show_paths = no`: hides file and network paths (Python, package folders,
  the ini) from pupils.
- `[projects] folder = H:\Documents`: the folder offered for every new project and in
  *Open Project* (pupils can still choose another one).

**Ini kept elsewhere (e.g. on a network share).** Start EduPython with
`--config <file>`, for example from a Start-menu shortcut:

```
Target:  \\server\apps\EduPython\EduPython.exe --config "\\server\config\edupython.ini"
```

An update then only replaces `EduPython.exe`; the ini stays where it is.
Relative paths in the ini (such as the booklet) are relative to the ini's folder.

**Crashes.** If EduPython ever crashes, it shows a message and saves the details to
`%LOCALAPPDATA%\EduPython\crash.log` (macOS: `~/Library/Application Support/EduPython/crash.log`).

Without an ini, a file called `booklet.pdf` next to the program is shown as the booklet.
Other places that are searched, in order: the file given with `--config`, the file in the environment variable
`EDUPYTHON_CONFIG`, the folder next to `EduPython.app` (macOS),
`%ProgramData%\EduPython` (Windows) and `/Library/Application Support/EduPython` (macOS).
To update the booklet for a new school year, replace the PDF. Nothing else changes.

## Project format

A project is just a folder of `.py` files. `edupython.json` remembers the main file and
the UML box positions. Any folder with Python files can be opened. Old tkinter projects
also open; `gui.py` then appears as a normal file.

## Develop

```bash
cargo run                 # debug build, uses an installed Python 3
cargo run -- path/to/project
cargo test                # parser, code generator, editor, console, runner tests
```

## Ready-to-run downloads (no installation for pupils)

The `Build EduPython` GitHub workflow (`.github/workflows/build.yml`)
builds:

- `EduPython-windows-x64.exe`: a **single portable exe**. Double-click and it runs. No installer
  and no Visual C++ runtime needed.
- `EduPython-macos-arm64.dmg` / `EduPython-macos-x64.dmg`: the app can be started directly
  from the DMG, or dragged to Applications.

Both have **Python 3.12 + pygame built in**. On first start the built-in Python is unpacked
once (a few seconds) into the user's local app-data folder
(`%LOCALAPPDATA%\EduPython` / `~/Library/Application Support/EduPython`).
Push a tag like `v2.0.0` to publish a release.

To build one locally:

```bash
python3 packaging/prepare_python.py aarch64-apple-darwin python-runtime.zip   # downloads Python, installs pygame
EDUPYTHON_PYTHON_ZIP="$PWD/python-runtime.zip" cargo build --release --features bundled-python
packaging/macos/make_dmg.sh target/release/edupython EduPython.dmg         # macOS only
```

Python builds come from [python-build-standalone](https://github.com/astral-sh/python-build-standalone).

**macOS note:** the DMG gets an ad-hoc signature only. On first launch pupils must
right-click ▸ Open (or allow it in System Settings ▸ Privacy & Security). To avoid that, set
`MACOS_SIGN_IDENTITY` to a Developer ID certificate and notarize the DMG.
