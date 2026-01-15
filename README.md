# libcdio-paranoia-rs

A Rust reimplementation of libcdio-paranoia for CD audio extraction with error correction.

## Features

- Drop-in replacement for the C libcdio-paranoia library
- Full C API compatibility via FFI
- Double-read verification for audio integrity
- Supports PARANOIA_MODE_FULL and PARANOIA_MODE_DISABLE

## Building

```bash
cargo build --release
```

This produces `target/release/libcdio_paranoia.so` (or `.dylib` on macOS).

## C API Usage

The library provides full API compatibility with libcdio-paranoia. Existing C programs can link against this library without modification.

### Compiling C programs

```bash
# With Rust library
gcc -o myprogram myprogram.c \
    -I /path/to/libcdio-paranoia-rs/include \
    -L /path/to/libcdio-paranoia-rs/target/release -lcdio_paranoia \
    -Wl,-rpath,/path/to/libcdio-paranoia-rs/target/release

# With system libcdio-paranoia (for comparison)
gcc -o myprogram myprogram.c \
    $(pkg-config --cflags --libs libcdio_paranoia)
```

### Example

See `examples/rip_track.c` for a complete example that rips a CD track to WAV.

```bash
# Build and run example with Rust library
cargo build
gcc -o /tmp/rip examples/rip_track.c \
    -I include \
    -L target/debug -lcdio_paranoia \
    -Wl,-rpath,$(pwd)/target/debug
/tmp/rip 1 /tmp/track1.wav
```

## API

The C API matches the original libcdio-paranoia:

```c
#include <cdio/paranoia/paranoia.h>

// Identify and open drive
cdrom_drive_t *drive = cdio_cddap_identify(NULL, CDDA_MESSAGE_FORGETIT, NULL);
cdio_cddap_open(drive);

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

## License

GPL-3.0 (same as original libcdio-paranoia)
