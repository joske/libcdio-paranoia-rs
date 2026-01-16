/*
 * cdio-paranoia - Rust implementation of libcdio-paranoia
 *
 * This header provides C API compatibility with the original libcdio-paranoia.
 * Link with -lcdio_paranoia
 */

#ifndef CDIO_PARANOIA_H
#define CDIO_PARANOIA_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque types */
typedef struct cdrom_drive_s cdrom_drive_t;
typedef struct cdrom_paranoia_s cdrom_paranoia_t;

/* LSN type (Logical Sector Number) */
typedef int32_t lsn_t;

/* Paranoia mode flags */
#define PARANOIA_MODE_DISABLE     0x00
#define PARANOIA_MODE_VERIFY      0x01
#define PARANOIA_MODE_FRAGMENT    0x02  /* Reserved */
#define PARANOIA_MODE_OVERLAP     0x04
#define PARANOIA_MODE_SCRATCH     0x08  /* Reserved */
#define PARANOIA_MODE_REPAIR      0x10  /* Reserved */
#define PARANOIA_MODE_NEVERSKIP   0x20
#define PARANOIA_MODE_FULL        0xFF

/* Callback events */
typedef enum {
    PARANOIA_CB_READ = 0,
    PARANOIA_CB_VERIFY,
    PARANOIA_CB_FIXUP_EDGE,
    PARANOIA_CB_FIXUP_ATOM,
    PARANOIA_CB_SCRATCH,
    PARANOIA_CB_REPAIR,
    PARANOIA_CB_SKIP,
    PARANOIA_CB_DRIFT,
    PARANOIA_CB_BACKOFF,
    PARANOIA_CB_OVERLAP,
    PARANOIA_CB_FIXUP_DROPPED,
    PARANOIA_CB_FIXUP_DUPED,
    PARANOIA_CB_READERR,
    PARANOIA_CB_CACHEERR,
    PARANOIA_CB_WROTE,
    PARANOIA_CB_FINISHED,
    PARANOIA_CB_WROTE_SILENCE
} paranoia_cb_mode_t;

/* Callback function type */
typedef void (*paranoia_callback_t)(long, paranoia_cb_mode_t);

/* Message destination */
#define CDDA_MESSAGE_FORGETIT  0
#define CDDA_MESSAGE_PRINTIT   1
#define CDDA_MESSAGE_LOGIT     2

/* ============================================================
 * CDDA Drive Functions (cdio_cddap_*)
 * ============================================================ */

/* Find and identify a CD-ROM drive */
cdrom_drive_t *cdio_cddap_find_a_cdrom(int messagedest, char **messages);
cdrom_drive_t *cdio_cddap_identify(const char *device, int messagedest, char **messages);
cdrom_drive_t *cdio_cddap_identify_cdio(void *p_cdio, int messagedest, char **messages);

/* Open/close drive */
int cdio_cddap_open(cdrom_drive_t *d);
void cdio_cddap_close(cdrom_drive_t *d);
void cdio_cddap_close_no_free_cdio(cdrom_drive_t *d);

/* Read audio data */
long cdio_cddap_read(cdrom_drive_t *d, void *buffer, lsn_t beginsector, long sectors);
long cdio_cddap_read_timed(cdrom_drive_t *d, void *buffer, lsn_t beginsector,
                           long sectors, int *milliseconds);

/* Disc/track information */
lsn_t cdio_cddap_disc_firstsector(cdrom_drive_t *d);
lsn_t cdio_cddap_disc_lastsector(cdrom_drive_t *d);
int cdio_cddap_tracks(cdrom_drive_t *d);
lsn_t cdio_cddap_track_firstsector(cdrom_drive_t *d, int track);
lsn_t cdio_cddap_track_lastsector(cdrom_drive_t *d, int track);
int cdio_cddap_track_channels(cdrom_drive_t *d, int track);
int cdio_cddap_track_audiop(cdrom_drive_t *d, int track);
int cdio_cddap_track_copyp(cdrom_drive_t *d, int track);
int cdio_cddap_track_preemp(cdrom_drive_t *d, int track);
int cdio_cddap_sector_gettrack(cdrom_drive_t *d, lsn_t sector);

/* Data endianness detection (1=big, 0=little, -1=unknown) */
int data_bigendianp(cdrom_drive_t *d);

/* Drive settings */
int cdio_cddap_speed_set(cdrom_drive_t *d, int speed);
void cdio_cddap_verbose_set(cdrom_drive_t *d, int err_action, int mes_action);

/* Messages */
char *cdio_cddap_messages(cdrom_drive_t *d);
char *cdio_cddap_errors(cdrom_drive_t *d);
void cdio_cddap_free_messages(char *messages);

/* ============================================================
 * Paranoia Functions (cdio_paranoia_*)
 * ============================================================ */

/* Initialize/free paranoia
 * Note: paranoia does NOT take ownership of the drive.
 * The caller must keep the drive alive while paranoia is in use,
 * and must free the drive separately after freeing paranoia.
 */
cdrom_paranoia_t *cdio_paranoia_init(cdrom_drive_t *d);
void cdio_paranoia_free(cdrom_paranoia_t *p);

/* Configuration */
void cdio_paranoia_modeset(cdrom_paranoia_t *p, int mode);
void cdio_paranoia_overlapset(cdrom_paranoia_t *p, long overlap);
void cdio_paranoia_set_range(cdrom_paranoia_t *p, long start, long end);
int cdio_paranoia_cachemodel_size(cdrom_paranoia_t *p, int sectors);

/* Seeking */
lsn_t cdio_paranoia_seek(cdrom_paranoia_t *p, int32_t seek, int whence);

/* Reading */
int16_t *cdio_paranoia_read(cdrom_paranoia_t *p, paranoia_callback_t callback);
int16_t *cdio_paranoia_read_limited(cdrom_paranoia_t *p, paranoia_callback_t callback,
                                     int max_retries);

/* Version - returns single null-terminated string */
const char *cdio_paranoia_version(void);
const char *cdio_cddap_version(void);

/* ============================================================
 * Legacy Aliases (without cdio_ prefix)
 * ============================================================ */

cdrom_drive_t *cdda_find_a_cdrom(int messagedest, char **messages);
cdrom_drive_t *cdda_identify(const char *device, int messagedest, char **messages);
int cdda_open(cdrom_drive_t *d);
void cdda_close(cdrom_drive_t *d);
long cdda_read(cdrom_drive_t *d, void *buffer, lsn_t beginsector, long sectors);
lsn_t cdda_disc_firstsector(cdrom_drive_t *d);
lsn_t cdda_disc_lastsector(cdrom_drive_t *d);
int cdda_tracks(cdrom_drive_t *d);
lsn_t cdda_track_firstsector(cdrom_drive_t *d, int track);
lsn_t cdda_track_lastsector(cdrom_drive_t *d, int track);

cdrom_paranoia_t *paranoia_init(cdrom_drive_t *d);
void paranoia_free(cdrom_paranoia_t *p);
void paranoia_modeset(cdrom_paranoia_t *p, int mode);
void paranoia_overlapset(cdrom_paranoia_t *p, long overlap);
void paranoia_set_range(cdrom_paranoia_t *p, long start, long end);
int paranoia_cachemodel_size(cdrom_paranoia_t *p, int sectors);
lsn_t paranoia_seek(cdrom_paranoia_t *p, int32_t seek, int whence);
int16_t *paranoia_read(cdrom_paranoia_t *p, paranoia_callback_t callback);
int16_t *paranoia_read_limited(cdrom_paranoia_t *p, paranoia_callback_t callback,
                                int max_retries);
const char *paranoia_version(void);

#ifdef __cplusplus
}
#endif

#endif /* CDIO_PARANOIA_H */
