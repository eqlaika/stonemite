# eqspell

`eqspell` parses passive EverQuest client spell metadata from `spells_us.txt`
and localized landing messages from `spells_us_str.txt`.

The crate is platform-neutral and does not inspect or communicate with a running
EverQuest process. Applications remain responsible for locating the client data
directory and deciding when to reload it.
