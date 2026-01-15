//! Test binary for CD drive access.
//!
//! Usage: cargo run --example cdtest [device] [--rip track output.wav]

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::uninlined_format_args,
    clippy::too_many_lines,
    clippy::unreadable_literal
)]

use std::{
    env,
    fs::File,
    io::{self, Write},
};

use cdio_paranoia::{CdromDrive, Paranoia, ParanoiaMode, CD_FRAMEWORDS};

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    // Parse arguments
    let mut device: Option<&str> = None;
    let mut rip_track: Option<u8> = None;
    let mut output_file: Option<&str> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--rip" => {
                if i + 2 >= args.len() {
                    eprintln!("Usage: --rip <track> <output.wav>");
                    std::process::exit(1);
                }
                rip_track = Some(args[i + 1].parse()?);
                output_file = Some(&args[i + 2]);
                i += 3;
            }
            "--help" | "-h" => {
                println!("Usage: cdtest [device] [--rip track output.wav]");
                println!();
                println!("Options:");
                println!("  device          CD device path (e.g., /dev/sr0)");
                println!("  --rip N FILE    Rip track N to FILE (WAV format)");
                println!("  --help          Show this help");
                return Ok(());
            }
            arg if !arg.starts_with('-') && device.is_none() => {
                device = Some(arg);
                i += 1;
            }
            _ => {
                eprintln!("Unknown argument: {}", args[i]);
                std::process::exit(1);
            }
        }
    }

    // Open drive
    println!("Opening CD drive...");
    let drive = CdromDrive::open(device)?;

    // Print disc info
    println!();
    println!("=== Disc Information ===");
    println!("Device: {:?}", drive.device_name);
    println!("Tracks: {}", drive.track_count());
    println!(
        "Audio range: sectors {} - {}",
        drive.disc_first_sector(),
        drive.disc_last_sector()
    );

    let total_sectors = drive.disc_last_sector() - drive.disc_first_sector() + 1;
    let total_seconds = total_sectors as f64 / 75.0; // 75 sectors per second
    let minutes = (total_seconds / 60.0) as i32;
    let seconds = total_seconds % 60.0;
    println!("Total length: {}:{:05.2}", minutes, seconds);

    println!();
    println!("=== Track List ===");
    for track in 1..=drive.track_count() {
        let first = drive.track_first_sector(track)?;
        let last = drive.track_last_sector(track)?;
        let is_audio = drive.track_is_audio(track);
        let length = last - first + 1;
        let track_seconds = length as f64 / 75.0;
        let track_min = (track_seconds / 60.0) as i32;
        let track_sec = track_seconds % 60.0;

        println!(
            "  Track {:2}: sectors {:6} - {:6} ({:2}:{:05.2}) {}",
            track,
            first,
            last,
            track_min,
            track_sec,
            if is_audio { "[AUDIO]" } else { "[DATA]" }
        );
    }

    // Test read
    println!();
    println!("=== Test Read ===");
    let mut paranoia = Paranoia::new(drive);

    // Try reading first audio sector
    paranoia.set_mode(ParanoiaMode::DISABLE); // Direct read for speed

    let sector_before = paranoia.cursor_sector();
    match paranoia.read() {
        Ok(samples) => {
            println!(
                "Read {} samples from sector {}",
                samples.len(),
                sector_before
            );

            // Calculate some stats
            let min = samples.iter().min().unwrap_or(&0);
            let max = samples.iter().max().unwrap_or(&0);
            let sum: i64 = samples.iter().map(|&s| s as i64).sum();
            let avg = sum / samples.len() as i64;
            let rms: f64 = (samples.iter().map(|&s| (s as f64).powi(2)).sum::<f64>()
                / samples.len() as f64)
                .sqrt();

            println!(
                "Sample stats: min={}, max={}, avg={}, RMS={:.1}",
                min, max, avg, rms
            );
        }
        Err(e) => {
            println!("Read failed: {}", e);
        }
    }

    // Rip track if requested
    if let (Some(track), Some(output)) = (rip_track, output_file) {
        println!();
        println!("=== Ripping Track {} ===", track);
        rip_track_to_wav(&mut paranoia, track, output)?;
    }

    Ok(())
}

fn rip_track_to_wav(
    paranoia: &mut Paranoia,
    track: u8,
    output_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let drive = paranoia.drive();

    if track == 0 || track > drive.track_count() {
        return Err(format!("Invalid track number: {}", track).into());
    }

    if !drive.track_is_audio(track) {
        return Err(format!("Track {} is not an audio track", track).into());
    }

    let first_sector = drive.track_first_sector(track)?;
    let last_sector = drive.track_last_sector(track)?;
    let total_sectors = (last_sector - first_sector + 1) as usize;
    let total_samples = total_sectors * CD_FRAMEWORDS;
    let total_bytes = total_samples * 2; // 16-bit samples

    println!(
        "Track {}: {} sectors, {} samples",
        track, total_sectors, total_samples
    );
    println!("Output: {}", output_path);

    // Seek to track start (offset from disc first sector)
    let disc_first = paranoia.drive().disc_first_sector();
    let seek_offset = first_sector - disc_first;
    let _ = paranoia.seek(seek_offset, std::io::SeekFrom::Start(0));

    // Set paranoia mode (FULL for error correction)
    paranoia.set_mode(ParanoiaMode::FULL);

    // Create WAV file
    let mut file = File::create(output_path)?;

    // Write WAV header
    write_wav_header(&mut file, total_bytes as u32)?;

    // Read and write sectors
    let mut sectors_read = 0;
    let start_time = std::time::Instant::now();

    while paranoia.cursor_sector() <= last_sector {
        match paranoia.read() {
            Ok(samples) => {
                // Write samples as little-endian bytes
                for &sample in samples {
                    file.write_all(&sample.to_le_bytes())?;
                }
                sectors_read += 1;

                // Progress update every 100 sectors
                if sectors_read % 100 == 0 {
                    let percent = (sectors_read * 100) / total_sectors;
                    let elapsed = start_time.elapsed().as_secs_f64();
                    let rate = sectors_read as f64 / elapsed;
                    print!(
                        "\rProgress: {}% ({} sectors, {:.1} sectors/sec)  ",
                        percent, sectors_read, rate
                    );
                    io::stdout().flush()?;
                }
            }
            Err(e) => {
                eprintln!("\nError at sector {}: {}", paranoia.cursor_sector(), e);
                // Continue anyway (paranoia will insert silence on skip)
            }
        }
    }

    let elapsed = start_time.elapsed();
    println!(
        "\rCompleted: {} sectors in {:.1}s ({:.1} sectors/sec)     ",
        sectors_read,
        elapsed.as_secs_f64(),
        sectors_read as f64 / elapsed.as_secs_f64()
    );

    Ok(())
}

fn write_wav_header(file: &mut File, data_size: u32) -> io::Result<()> {
    // RIFF header
    file.write_all(b"RIFF")?;
    file.write_all(&(data_size + 36).to_le_bytes())?; // File size - 8
    file.write_all(b"WAVE")?;

    // fmt chunk
    file.write_all(b"fmt ")?;
    file.write_all(&16u32.to_le_bytes())?; // Chunk size
    file.write_all(&1u16.to_le_bytes())?; // Audio format (PCM)
    file.write_all(&2u16.to_le_bytes())?; // Channels (stereo)
    file.write_all(&44100u32.to_le_bytes())?; // Sample rate
    file.write_all(&176_400_u32.to_le_bytes())?; // Byte rate (44100 * 2 * 2)
    file.write_all(&4u16.to_le_bytes())?; // Block align (2 * 2)
    file.write_all(&16u16.to_le_bytes())?; // Bits per sample

    // data chunk
    file.write_all(b"data")?;
    file.write_all(&data_size.to_le_bytes())?;

    Ok(())
}
