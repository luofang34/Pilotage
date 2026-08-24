#pragma once

typedef void* XPLMDataRef;
typedef int XPLMDataTypeID;

enum {
    xplmType_Unknown = 0,
    xplmType_Int = 1,
    xplmType_Float = 2,
    xplmType_Double = 4,
    xplmType_FloatArray = 8,
    xplmType_IntArray = 16,
    xplmType_Data = 32,
};

typedef int (*XPLMGetDatai_f)(void*);
typedef void (*XPLMSetDatai_f)(void*, int);
typedef float (*XPLMGetDataf_f)(void*);
typedef void (*XPLMSetDataf_f)(void*, float);
typedef double (*XPLMGetDatad_f)(void*);
typedef void (*XPLMSetDatad_f)(void*, double);
typedef int (*XPLMGetDatavi_f)(void*, int*, int, int);
typedef void (*XPLMSetDatavi_f)(void*, int*, int, int);
typedef int (*XPLMGetDatavf_f)(void*, float*, int, int);
typedef void (*XPLMSetDatavf_f)(void*, float*, int, int);
typedef int (*XPLMGetDatab_f)(void*, void*, int, int);
typedef void (*XPLMSetDatab_f)(void*, void*, int, int);

#ifdef __cplusplus
extern "C" {
#endif

XPLMDataRef XPLMFindDataRef(const char* name);
int XPLMCanWriteDataRef(XPLMDataRef dataref);
XPLMDataTypeID XPLMGetDataRefTypes(XPLMDataRef dataref);
int XPLMGetDatai(XPLMDataRef dataref);
void XPLMSetDatai(XPLMDataRef dataref, int value);
float XPLMGetDataf(XPLMDataRef dataref);
void XPLMSetDataf(XPLMDataRef dataref, float value);
int XPLMGetDatavf(XPLMDataRef dataref, float* values, int offset,
                  int maximum);
void XPLMSetDatavf(XPLMDataRef dataref, float* values, int offset, int count);
XPLMDataRef XPLMRegisterDataAccessor(
    const char* name, XPLMDataTypeID type, int writable,
    XPLMGetDatai_f read_int, XPLMSetDatai_f write_int,
    XPLMGetDataf_f read_float, XPLMSetDataf_f write_float,
    XPLMGetDatad_f read_double, XPLMSetDatad_f write_double,
    XPLMGetDatavi_f read_int_array, XPLMSetDatavi_f write_int_array,
    XPLMGetDatavf_f read_float_array, XPLMSetDatavf_f write_float_array,
    XPLMGetDatab_f read_data, XPLMSetDatab_f write_data, void* read_refcon,
    void* write_refcon);
void XPLMUnregisterDataAccessor(XPLMDataRef dataref);

#ifdef __cplusplus
}
#endif
