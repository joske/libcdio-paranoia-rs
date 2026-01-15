/* Rip a CD track to WAV using cdio-paranoia Rust library */

#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <cdio/paranoia/paranoia.h>

#define CD_FRAMESAMPLES 588
#define SAMPLE_RATE 44100
#define CHANNELS 2
#define BITS_PER_SAMPLE 16

static void write_wav_header(FILE *f, uint32_t num_samples) {
    uint32_t data_size = num_samples * CHANNELS * (BITS_PER_SAMPLE / 8);
    uint32_t file_size = 36 + data_size;
    uint16_t block_align = CHANNELS * (BITS_PER_SAMPLE / 8);
    uint32_t byte_rate = SAMPLE_RATE * block_align;

    /* RIFF header */
    fwrite("RIFF", 1, 4, f);
    fwrite(&file_size, 4, 1, f);
    fwrite("WAVE", 1, 4, f);

    /* fmt chunk */
    fwrite("fmt ", 1, 4, f);
    uint32_t fmt_size = 16;
    fwrite(&fmt_size, 4, 1, f);
    uint16_t audio_format = 1; /* PCM */
    fwrite(&audio_format, 2, 1, f);
    uint16_t num_channels = CHANNELS;
    fwrite(&num_channels, 2, 1, f);
    uint32_t sample_rate = SAMPLE_RATE;
    fwrite(&sample_rate, 4, 1, f);
    fwrite(&byte_rate, 4, 1, f);
    fwrite(&block_align, 2, 1, f);
    uint16_t bits = BITS_PER_SAMPLE;
    fwrite(&bits, 2, 1, f);

    /* data chunk */
    fwrite("data", 1, 4, f);
    fwrite(&data_size, 4, 1, f);
}

int main(int argc, char **argv) {
    int track_num = 1;
    const char *output = "/tmp/track_c.wav";

    if (argc > 1) track_num = atoi(argv[1]);
    if (argc > 2) output = argv[2];

    printf("Ripping track %d to %s\n", track_num, output);

    /* Identify and open drive */
    cdrom_drive_t *drive = cdio_cddap_identify(NULL, CDDA_MESSAGE_FORGETIT, NULL);
    if (!drive) {
        fprintf(stderr, "Failed to identify drive\n");
        return 1;
    }

    if (cdio_cddap_open(drive) != 0) {
        fprintf(stderr, "Failed to open drive\n");
        cdio_cddap_close(drive);
        return 1;
    }

    int tracks = cdio_cddap_tracks(drive);
    if (track_num < 1 || track_num > tracks) {
        fprintf(stderr, "Invalid track %d (disc has %d tracks)\n", track_num, tracks);
        cdio_cddap_close(drive);
        return 1;
    }

    lsn_t first = cdio_cddap_track_firstsector(drive, track_num);
    lsn_t last = cdio_cddap_track_lastsector(drive, track_num);
    long total_sectors = last - first + 1;

    printf("Track %d: sectors %d - %d (%ld sectors)\n", track_num, first, last, total_sectors);

    /* Init paranoia */
    cdrom_paranoia_t *paranoia = cdio_paranoia_init(drive);
    if (!paranoia) {
        fprintf(stderr, "Failed to init paranoia\n");
        return 1;
    }

    cdio_paranoia_modeset(paranoia, PARANOIA_MODE_FULL);

    /* Seek to track start */
    lsn_t disc_first = cdio_cddap_disc_firstsector(drive);
    /* Note: paranoia positions are relative to disc start */
    cdio_paranoia_seek(paranoia, first - disc_first, 0);

    /* Open output file */
    FILE *f = fopen(output, "wb");
    if (!f) {
        fprintf(stderr, "Failed to open %s\n", output);
        cdio_paranoia_free(paranoia);
        return 1;
    }

    /* Write placeholder header (will update at end) */
    write_wav_header(f, 0);

    /* Rip */
    printf("Ripping with FULL paranoia mode...\n");
    long sectors_read = 0;
    uint32_t total_samples = 0;

    while (sectors_read < total_sectors) {
        int16_t *samples = cdio_paranoia_read(paranoia, NULL);
        if (!samples) {
            fprintf(stderr, "\nRead error at sector %ld\n", sectors_read);
            break;
        }

        fwrite(samples, sizeof(int16_t), CD_FRAMESAMPLES * CHANNELS, f);
        total_samples += CD_FRAMESAMPLES;
        sectors_read++;

        if (sectors_read % 100 == 0) {
            printf("\r  %ld / %ld sectors (%.1f%%)",
                   sectors_read, total_sectors,
                   100.0 * sectors_read / total_sectors);
            fflush(stdout);
        }
    }

    printf("\r  %ld / %ld sectors (100.0%%)\n", sectors_read, total_sectors);

    /* Update WAV header with actual size */
    fseek(f, 0, SEEK_SET);
    write_wav_header(f, total_samples);
    fclose(f);

    cdio_paranoia_free(paranoia);
    cdio_cddap_close(drive);

    printf("Done! Wrote %s\n", output);
    return 0;
}
