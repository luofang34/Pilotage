#pragma once

typedef float (*XPLMFlightLoop_f)(float, float, int, void*);

#ifdef __cplusplus
extern "C" {
#endif

void XPLMRegisterFlightLoopCallback(XPLMFlightLoop_f callback, float interval,
                                    void* refcon);
void XPLMUnregisterFlightLoopCallback(XPLMFlightLoop_f callback, void* refcon);

#ifdef __cplusplus
}
#endif
