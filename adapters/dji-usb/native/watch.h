#ifndef PILOTAGE_DJI_WATCH_H
#define PILOTAGE_DJI_WATCH_H
#include "logiclink.h"
typedef struct DjiWatch DjiWatch;
int dji_watch_open(DjiWatch **output, DjiError *error);
int dji_watch_poll(DjiWatch *watch);
void dji_watch_close(DjiWatch *watch);
#endif
