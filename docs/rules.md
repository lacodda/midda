# Rules

midda finds what can go by rules: a rule names a folder or a file, says what
brings it back and what is lost, and how careful to be. The rules midda ships
are files in [`crates/midda-core/rules/`](../crates/midda-core/rules/), and
your own are files of the same format in midda's rules folder. Whatever a
shipped rule can say, yours can - and a file of yours can become a shipped one
by a pull request.

## Where your rules go

Every `.toml` file in

```
%APPDATA%\com.lacodda.midda\rules
```

is read when midda starts and whenever **Read again** is pressed under
*Your rules* on the *Safe to delete* page. **Open the folder** opens it in
Explorer; **Add a file…** copies a file in once it reads with the rest; **Save
a copy…** hands one on; **Remove…** sends one to the Recycle Bin.

Your files are tried before midda's, in the order of their names: on your
disk your word comes first. A file that does not read is listed with why, and
costs only itself. An `id` names one rule or place across every file, yours
and midda's: to change what a shipped rule says, write your own with another
id - it is tried first.

`kept.toml` is the window's own: **Keep** on a finding writes a keep there,
and **Stop keeping** takes it out. Edit it as any other file; once it holds
anything but keeps, the window leaves it to you.

## Keeping what is never to go

```toml
[[keep]]
paths = ["D:/Projects/app/target"]
why = "the release build I hand out"
```

Nothing a keep names is a finding, nor anything in it. A folder that holds
something kept is never taken whole: keep `target/release/app.exe` and the
`target` around it stays one thing nobody deletes. `why` is optional, and
shown beside the keep.

## A rule

```toml
[[rule]]
id = "unity-library"
name = "Unity library"
group = "build"
paths = ["Library"]
beside = ["Assets", "ProjectSettings"]

[[rule.restore]]
risk = "safe"
explain = "What Unity imported from the project's assets. Opening the project imports them again, which takes a while."
```

| Field | What it says |
| --- | --- |
| `id` | Lowercase letters, digits and dashes, unique across every file. |
| `name` | What a reader calls the thing found. |
| `group` | Where it is listed: `dependencies`, `build`, `caches`, `browser-caches`, `recycle-bin`, `temporary`, `windows`, `crash-reports`, `downloads`, `leftovers`. |
| `paths` | Where it is, read from its end (see below). |
| `beside` | It counts only when one of these is next to it. |
| `inside` | It counts only when one of these is in it. |
| `closed` | Nothing inside what its paths name is looked at by any rule, even when this one does not apply - what is in a `node_modules` is installed packages, never a project's build. |
| `entries` | `folders` (the default), `files` or `all`. |
| `older` | Only what nothing in has been written to for that long: `7 days`, `2 weeks`, `3 months`, `1 year`. |
| `goes` | `whole` (the default) or `contents`: the folder stays, what is in it goes - with `older`, only what is that old. |
| `hollow` | Only a folder with no file anywhere in it. |
| `within` | Only strictly inside what these consumers hold: `documents`, `media`, `unrecognised`, … |
| `except` | Paths the rule never names, and that never go with a folder it empties. |
| `check` | What only the file can say, asked after the walk: `shortcut` (only a shortcut to nothing is the rule's) or `installer` (whether what it installs is installed). |
| `marker` | A file the folder holds to say what it is, and how it starts (below). |
| `about` | What more is read of a finding once it is one: `recycle-bin`. |
| `restore` | The ways back, tried in order (below). At least one. |
| `cleanup` | A cache's own tool, run instead of deleting the folder - see `rules/dev.toml`. |

A rule that cannot explain itself cannot be written: one with no way back,
or whose last way back has a condition, is refused when the file is read.

### Ways back

```toml
[[rule.restore]]
when = "pnpm-lock.yaml"
command = "pnpm install"
risk = "safe"
explain = "Packages installed for this project. The lockfile beside it pins every version, so an install brings back exactly these."

[[rule.restore]]
command = "npm install"
risk = "caution"
explain = "Packages installed for this project. There is no lockfile beside it, so an install may bring newer versions than the ones here."
```

The first way whose `when` is next to the folder is taken - or, for a rule
with a check, whose `if` is what the check said (`installed`,
`older-installed`); the last has neither and takes the rest. `risk` is `safe`
(nothing anyone needs is lost) or `caution` (it may not come back as it was);
`command` is what to run beside the folder to make it again; `explain` is one
or two sentences said to the reader as they are.

### Paths

A path is names separated by `/`, read from its end: `node_modules` is one
anywhere, `AppData/Local/npm-cache` one in any profile. `*` and `?` work inside
a name, `**` stands for any number of folders, and case is ignored.

- A leading `/` puts the first name at the top of a volume: `/Windows` is the
  system's folder, not every folder called Windows.
- A leading drive puts it at the top of that volume only: `D:/Archive`.
- A leading known folder puts it where the shell keeps that folder:
  `{downloads}/*.iso`. The known folders are `{desktop}`, `{documents}`,
  `{downloads}`, `{music}`, `{pictures}` and `{videos}`; each is every
  profile's own folder and wherever your account's shell moved it - Downloads
  on `D:`, the Desktop in OneDrive.

### A marker

```toml
[rule.marker]
file = "CACHEDIR.TAG"
starts = "Signature: 8a477f597d28d172789f06886806bc55"
```

Only a folder that holds `file`, and whose `file` starts with `starts`, is the
rule's. The file is read after the walk, as far as `starts` is long - at most
512 bytes. This is how midda finds a cache that says so itself, by the
[cache directory tagging standard](https://bford.info/cachedir/).

## Places

A place says what something is without saying it can go, so a disk can be
told as a handful of consumers - see the header of
[`rules/places.toml`](../crates/midda-core/rules/places.toml):

```toml
[[place]]
id = "my-stash"
name = "my stash"
consumer = "media"
paths = ["D:/Stash"]
```

## Sending a rule to midda

A rule that would help anyone - a tool's cache, a build folder midda does not
know - belongs in `crates/midda-core/rules/`. See
[CONTRIBUTING](../CONTRIBUTING.md#adding-a-rule).
