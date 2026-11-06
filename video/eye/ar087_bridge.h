#pragma once
#include <windows.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef struct EyeAr087 EyeAr087;
// Only called AFTER the owner verifies and locks its pinned runtime files.
int eye_ar087_open(HMODULE ar, HMODULE cv, HMODULE cuda, const char* models,
    unsigned width, unsigned height, EyeAr087** out);
// 0: corrected, 1: no reliable face, negative: SDK/adapter failure. No fallback.
int eye_ar087_process(EyeAr087*, const unsigned char* bgr, unsigned char* corrected);
int eye_ar087_close(EyeAr087*); // 0 means cleanup completed
#ifdef __cplusplus
}
#endif
