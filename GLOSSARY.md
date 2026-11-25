# sshexplorer

A local desktop app for one operator to browse remote filesystems over SSH.

## Language

**Operator**:
The person who runs sshexplorer on their own machine and connects to hosts they already have credentials for.
_Avoid_: User, client

**Host**:
A target the operator can save. It records a display name, an address, a port, an account, an authentication method, an optional key file, whether its Sessions are readonly, whether it remembers a password or key passphrase, the local folder where downloads are written, and the initial remote folder its Sessions open in. A Host runs Linux, macOS, or a BSD. Several Sessions may be opened from one Host. The same fields can open a Session that is not saved.
_Avoid_: Server, machine, connection

**Authentication method**:
How a Host's Sessions log in: Automatic, Password, Key file, or Agent. Automatic tries the key file, then the agent's other keys, then a password. The other choices use only that one. A new Host uses Automatic.
_Avoid_: Auth type, login mode

**OpenSSH config**:
The operator's own `ssh` client configuration. A Session looks up its address there as `ssh` would, and the config supplies any account, key file, or port the Host leaves empty. A value set on the Host wins.
_Avoid_: ssh_config, SSH settings

**Known hosts**:
The operator's record of host keys they have trusted, shared with `ssh`. A Session to a host whose key is not there asks the operator to trust it. A Session to a host whose key has changed does not open.
_Avoid_: Fingerprint store, trusted hosts

**Key file**:
A path on a Host to a private key. When the path is set, a Session offers that key.
_Avoid_: Identity file, PEM

**Session**:
One authenticated SSH connection to one host, shown as one tab. It has its own connection, which closes with it. A Session opens in Tree view, and the operator can switch that Session to Folder view. Several Sessions may target the same host or different hosts.
_Avoid_: Tab, connection

**Tab**:
The control that selects a Session.

**Tree view**:
The browsing view that shows the remote filesystem as an expandable tree rooted at `/`. A Session opens in Tree view.
_Avoid_: Full tree

**Folder view**:
The browsing view that lists one directory's entries, including `.` and `..`.
_Avoid_: List view, current folder

**Readonly**:
A setting on a Host that its Sessions inherit. A readonly Session cannot change the remote filesystem. Download stays available. A new Host is readonly.
_Avoid_: Locked

**Focused entry**:
The one entry in a listing that the keyboard is on. Preview shows it.
_Avoid_: Cursor, current entry

**Preview**:
The panel beside the listing that shows the Focused entry’s contents when its file type supports preview. An entry’s metadata is available from Properties in its context menu.
_Avoid_: Viewer, inspector

**Scratch folder**:
Where Preview keeps the bytes it has fetched from a host: in memory, the system temp directory, or a directory the operator chooses. Nothing kept there outlives the Preview that fetched it.
_Avoid_: Cache, temp dir

**Selection**:
The set of entries a Download acts on. It holds one or more entries that share one parent directory. Selecting an entry under a different parent starts a new Selection.
_Avoid_: Marked entries, checked entries

**Download**:
A copy of each entry in the Selection, with a directory's contents, written into the Host's local folder.
_Avoid_: Transfer, export

**Local folder**:
The directory on the operator's machine that receives a Download. It is set on the Host and defaults to the operator's Downloads directory.
_Avoid_: Download directory, destination

**Initial remote folder**:
The remote directory a Session opens in. It is set on the Host. The operator can navigate above it. Empty means the account's home directory.
_Avoid_: Start folder, remote root

**Current directory**:
The directory a Session is at. In Folder view it is the listed directory. In Tree view it is the Focused entry when that is a directory, or the Focused entry's parent, or the Initial remote folder when nothing is focused. A Search starts there.
_Avoid_: Current place, working directory, cwd

**Refresh**:
The operator's request that a Session fetch the Current directory again from the host, and in Tree view the expanded directories below it. A Session's listings change only on a Refresh, an Auto-refresh, or when a Match is opened in a directory that no longer lists it.
_Avoid_: Reload, sync

**Auto-refresh**:
A Refresh a Session runs by itself at an interval the operator chooses, off by default. It does not run while the Session is out of view or showing Matches.
_Avoid_: Polling, watch

**Disconnected**:
A Session whose connection has dropped. It keeps its tab and its listings, which can be browsed but not refreshed, searched, or downloaded from, and it returns to normal when it reconnects.
_Avoid_: Offline, dead

**Search**:
A lookup of file and directory names, run on the host at and below the Session's current directory.
_Avoid_: find, grep, content search

**Match**:
An entry whose name a Search found. Opening a Match ends the Search and moves the Session to it.
_Avoid_: Hit, result

**Remember**:
A setting on a saved Host, off by default, that keeps a password or key passphrase for later Sessions of that Host. While it is off, that secret lasts only for the Session that asked for it. Saving the Host with it off removes the kept secret.
_Avoid_: Save password, master password
