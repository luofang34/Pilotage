#ifndef PILOTAGE_DJI_WATCH_H
#define PILOTAGE_DJI_WATCH_H
#include "logiclink.h"
typedef struct DjiWatch DjiWatch;
int dji_watch_open(DjiWatch **output, DjiError *error);
int dji_watch_access_open(int ground_recovery, DjiWatch **output, DjiError *error);
int dji_watch_poll(DjiWatch *watch);
uint32_t dji_watch_access_status(DjiWatch *watch);
void dji_watch_access_permit(DjiWatch *watch, int permitted);
void dji_watch_close(DjiWatch *watch);
#endif
