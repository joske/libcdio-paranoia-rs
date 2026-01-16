# libcdio-paranoia-rs

A Rust reimplementation of libcdio-paranoia for CD audio extraction with error correction.

## Features

- Drop-in replacement for the C libcdio-paranoia library
- Full C API compatibility via FFI
- Multi-stage verification for audio integrity:
  - Cache-based verification against previous reads
  - Double-read verification with jitter detection
  - Multi-read consensus voting (3-5 reads) for persistent mismatches
- Per-sample verification tracking with confidence flags
- Automatic jitter correction (sample offset detection and alignment)
- Rift analysis for detecting dropped/duplicated samples
- Exponential backoff for persistent read errors
- Supports all standard paranoia modes

## Requirements

- Rust 1.70+
- libcdio (for the `libcdio` feature)

On Arch Linux:
```bash
pacman -S libcdio
```

On Debian/Ubuntu:
```bash
apt install libcdio-dev
```

## Building

```bash
cargo build --release
```

This produces `target/release/libcdio_paranoia.so` (or `.dylib` on macOS).

## Rust API Usage

```rust
use cdio_paranoia::{CdromDrive, Paranoia, ParanoiaMode};

// Open drive
let drive = CdromDrive::open(None)?; // None = default device

// Initialize paranoia
let mut paranoia = Paranoia::new(drive);
paranoia.set_mode(ParanoiaMode::FULL);

// Seek to track start
paranoia.seek(first_sector, SeekFrom::Start(0));

// Read sectors
while let Ok(samples) = paranoia.read() {
    // samples is &[i16] with 1176 samples (one CD frame)
    process_audio(samples);
}
```

## C API Usage

The library provides full API compatibility with libcdio-paranoia. Existing C programs can link against this library without modification.

### Compiling C programs

The same source file can be compiled against either library:

```bash
# With Rust library
gcc -o myprogram myprogram.c \
    -I /path/to/libcdio-paranoia-rs/include \
    -L /path/to/libcdio-paranoia-rs/target/release -lcdio_paranoia \
    -Wl,-rpath,/path/to/libcdio-paranoia-rs/target/release

# With system libcdio-paranoia
gcc -o myprogram myprogram.c \
    $(pkg-config --cflags --libs libcdio_paranoia)
```

### Example

See `examples/rip_track.c` for a complete example that rips a CD track to WAV.

```bash
# Build and run example
cargo build
gcc -o /tmp/rip examples/rip_track.c \
    -I include \
    -L target/debug -lcdio_paranoia \
    -Wl,-rpath,$(pwd)/target/debug
/tmp/rip 1 /tmp/track1.wav
```

### C API Reference

```c
#include <cdio/paranoia/paranoia.h>

// Identify and open drive
cdrom_drive_t *drive = cdio_cddap_identify(NULL, CDDA_MESSAGE_FORGETIT, NULL);
cdio_cddap_open(drive);

// Query disc info
int tracks = cdio_cddap_tracks(drive);
lsn_t first = cdio_cddap_track_firstsector(drive, 1);
lsn_t last = cdio_cddap_track_lastsector(drive, 1);

// Initialize paranoia
cdrom_paranoia_t *paranoia = cdio_paranoia_init(drive);
cdio_paranoia_modeset(paranoia, PARANOIA_MODE_FULL);

// Seek and read
cdio_paranoia_seek(paranoia, sector, SEEK_SET);
int16_t *samples = cdio_paranoia_read(paranoia, NULL);

// Cleanup (paranoia does NOT own drive - free separately)
cdio_paranoia_free(paranoia);
cdio_cddap_close(drive);
```

## Paranoia Modes

| Mode | Value | Description |
|------|-------|-------------|
| `PARANOIA_MODE_DISABLE` | 0x00 | No verification, fastest |
| `PARANOIA_MODE_VERIFY` | 0x01 | Verify data integrity in overlap area |
| `PARANOIA_MODE_OVERLAP` | 0x04 | Perform overlapped reads |
| `PARANOIA_MODE_NEVERSKIP` | 0x20 | Never skip failed reads |
| `PARANOIA_MODE_FULL` | 0xFF | Maximum paranoia (all flags) |

## How It Works

This implementation uses a multi-stage verification algorithm:

### Stage 1: Cache Verification
1. Read a batch of sectors (26 sectors) with overlap
2. Check if data matches any cached previous reads using sorted index lookup
3. If match found, data is verified without additional reads

### Stage 2: Double-Read Verification
4. If no cache match, perform a second read of the same region
5. Compare the two reads with jitter detection (handles sample offset errors)
6. If they match (with or without jitter), data is verified

### Stage 3: Consensus Voting
7. If double-read fails, collect up to 5 reads total
8. Perform per-sample majority voting to determine correct values
9. Track confidence level for each sample (50% = verified, 75% = high confidence)

### Error Recovery
- **Jitter correction**: Detects and corrects sample offsets between reads
- **Rift analysis**: Detects dropped or duplicated samples and repairs them
- **Backoff mechanism**: On persistent errors, seeks away from problem area, waits, then retries with exponential backoff (16→32→64→128 sectors, 50→500ms delay)

### Per-Sample Tracking
Each sample is tracked with verification flags:
- `VERIFIED`: Sample confirmed by multiple matching reads
- `EDGE`: Sample at fragment boundary (needs extra verification)
- `BLANKED`: No data available (filled with silence)

This approach provides reliable error detection while maintaining good performance through batch reading and intelligent caching.

## Module Structure

| Module | Description |
|--------|-------------|
| `paranoia` | Core state machine for verified reading |
| `cdda` | CD-ROM drive interface and audio reading |
| `block` | Data structures for audio blocks and fragments |
| `consensus` | Multi-read consensus voting algorithm |
| `backoff` | Exponential backoff for persistent errors |
| `overlap` | Overlap detection and dynamic adjustment |
| `isort` | Sorted index for O(1) sample lookup |
| `gap` | Silence detection and rift analysis |
| `ffi` | C API compatibility layer |

## Compatibility

The library produces bit-identical output to the original libcdio-paranoia when ripping the same disc. This has been verified by comparing MD5 checksums of ripped audio.

## License

GPL-3.0 (same as original libcdio-paranoia)
