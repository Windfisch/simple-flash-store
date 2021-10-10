simple-flash-store
==================

This crate is a very simple library allowing you to store data in your microcontroller's
flash memory without wearing it out too quickly.

The basic concept revolves around "files" which are identified only by a number between 0
and 254, that can contain any amount of data bytes, regardless of your flash page size.

Advantages:
	- Low overhead (only 4 bytes or your flash's word size, whichever is larger).
	- Perfect wear leveling: All pages have the same erase count at any time.
	- Small code size.
	- No limits on file size.
	- Easy to use.
	- Well-suited for small flash memories.

Disadvantages:
	- No data safety during write operations: A power loss during writing can make it necessary
	  to reformat the memory, losing all data.
	- Locating a file is slow (requires reading once over the whole flash).
	- Not suited for large memory sizes or large write throughput.

Rationale
---------

While microcontrollers used to offer a built-in EEPROM supporting more than 100k write cycles,
modern chips such as STM32 tend to replace this by their flash self-programming capabilities.
Flash memories, however, only support 10k or less write cycles, and can reach their end of life
considerably faster (3 years at 10 write/erase cycles per day). Additionally, flash cannot be
reprogrammed as fine grained, so you need to erase and reprogram one or more kilobytes at once
typically. This wears the memory cells out even quicker if naive "read, modify in memory, erase,
write back" cycles are used for lots of small variables.

Wear-leveling schemes exist that distribute the erase cycles equally on all available flash pages.
Also, while flash can only be erased page-wise, it can be written with word granularity. By
exploiting this, a page can be written to multiple times before it needs to be erased again.

Principle of operation
----------------------

Each file entry consists of a 4-byte header, followed by the file contents plus enough padding
bytes so the entry's size is a multiple of the word size.

Files are written sequentially to the flash area. "Overwriting" a file is done by just appending
an updated version of the file to the back of the store. If the store becomes full, all data is
compacted by erasing all pages and writing only the last (up-to-date) versions of all files back
to flash.

While this simple scheme allows operation without an unused, spare flash page (saving valuable
memory), this implies that finding a file to read or finding the end to append requires reading
all the flash in the worst case.
