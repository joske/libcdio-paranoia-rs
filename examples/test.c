/* Simple test program for cdio-paranoia Rust library */

#include <stdio.h>
#include <stdlib.h>
#include "../include/cdio/paranoia/paranoia.h"

int main(int argc, char **argv) {
    const char *device = argc > 1 ? argv[1] : NULL;

    printf("cdio-paranoia Rust library test\n");
    printf("================================\n\n");

    /* Print version */
    const char *ver = cdio_paranoia_version();
    if (ver) {
        printf("Version: %s\n\n", ver);
    }

    /* Identify drive */
    printf("Opening drive: %s\n", device ? device : "(default)");
    cdrom_drive_t *drive = cdio_cddap_identify(device, CDDA_MESSAGE_FORGETIT, NULL);
    if (!drive) {
        fprintf(stderr, "Failed to identify drive\n");
        return 1;
    }
    printf("Drive identified successfully\n");

    /* Open the drive */
    if (cdio_cddap_open(drive) != 0) {
        fprintf(stderr, "Failed to open drive\n");
        cdio_cddap_close(drive);
        return 1;
    }
    printf("Drive opened successfully\n");

    /* Print disc info */
    int tracks = cdio_cddap_tracks(drive);
    lsn_t first = cdio_cddap_disc_firstsector(drive);
    lsn_t last = cdio_cddap_disc_lastsector(drive);

    printf("Tracks: %d\n", tracks);
    printf("First sector: %d\n", first);
    printf("Last sector: %d\n", last);
    printf("\nTrack list:\n");

    for (int t = 1; t <= tracks; t++) {
        lsn_t tfirst = cdio_cddap_track_firstsector(drive, t);
        lsn_t tlast = cdio_cddap_track_lastsector(drive, t);
        int is_audio = cdio_cddap_track_audiop(drive, t);
        printf("  Track %2d: %6d - %6d %s\n", t, tfirst, tlast,
               is_audio ? "[AUDIO]" : "[DATA]");
    }

    /* Initialize paranoia */
    printf("\nInitializing paranoia...\n");
    cdrom_paranoia_t *paranoia = cdio_paranoia_init(drive);
    if (!paranoia) {
        fprintf(stderr, "Failed to init paranoia\n");
        cdio_cddap_close(drive);
        return 1;
    }

    /* Set mode */
    cdio_paranoia_modeset(paranoia, PARANOIA_MODE_FULL);

    /* Read one sector */
    printf("Reading sector %d...\n", first);
    int16_t *samples = cdio_paranoia_read(paranoia, NULL);
    if (samples) {
        printf("Read 1176 samples successfully\n");

        /* Calculate simple stats */
        int16_t min = samples[0], max = samples[0];
        long sum = 0;
        for (int i = 0; i < 1176; i++) {
            if (samples[i] < min) min = samples[i];
            if (samples[i] > max) max = samples[i];
            sum += samples[i];
        }
        printf("Sample stats: min=%d, max=%d, avg=%ld\n", min, max, sum / 1176);
    } else {
        printf("Read failed\n");
    }

    /* Cleanup - paranoia does NOT own the drive, so free separately */
    printf("Freeing paranoia...\n");
    cdio_paranoia_free(paranoia);
    printf("Closing drive...\n");
    cdio_cddap_close(drive);

    printf("\nDone!\n");
    return 0;
}
