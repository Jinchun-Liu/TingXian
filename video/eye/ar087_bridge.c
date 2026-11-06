// Compiled as C: official NvCVImage C++ constructors otherwise call unpinned
// import/proxy functions. All calls below use the explicitly loaded modules.
#include "ar087_bridge.h"
#include <nvAR.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>

struct EyeAr087 {
    NvCV_Status (*version)(unsigned*);
    NvCV_Status (*create)(NvAR_FeatureID, NvAR_FeatureHandle*);
    NvCV_Status (*load)(NvAR_FeatureHandle);
    NvCV_Status (*run)(NvAR_FeatureHandle);
    NvCV_Status (*destroy)(NvAR_FeatureHandle);
    NvCV_Status (*stream_create)(CUstream*);
    NvCV_Status (*stream_destroy)(CUstream);
    NvCV_Status (*u32)(NvAR_FeatureHandle, const char*, unsigned);
    NvCV_Status (*s32)(NvAR_FeatureHandle, const char*, int);
    NvCV_Status (*object)(NvAR_FeatureHandle, const char*, void*, unsigned long);
    NvCV_Status (*string)(NvAR_FeatureHandle, const char*, const char*);
    NvCV_Status (*stream_set)(NvAR_FeatureHandle, const char*, CUstream);
    NvCV_Status (*array)(NvAR_FeatureHandle, const char*, float*, int);
    NvCV_Status (*alloc)(NvCVImage*, unsigned, unsigned, NvCVImage_PixelFormat, NvCVImage_ComponentType, unsigned, unsigned, unsigned);
    NvCV_Status (*init)(NvCVImage*, unsigned, unsigned, int, void*, NvCVImage_PixelFormat, NvCVImage_ComponentType, unsigned, unsigned);
    NvCV_Status (*transfer)(const NvCVImage*, NvCVImage*, float, CUstream, NvCVImage*);
    void (*dealloc)(NvCVImage*);
    int (__stdcall *sync)(CUstream);
    NvAR_FeatureHandle feature;
    CUstream stream;
    NvCVImage input, output, temporary;
    NvAR_Point2f landmarks[68], gaze_landmarks[12];
    float confidence[68], gaze_angles[2], translation[3];
    NvAR_Quaternion pose;
    NvAR_Point3f direction[2];
    NvAR_Rect box;
    NvAR_BBoxes boxes;
    unsigned width, height;
};
#define RESOLVE(field, module, name) do { \
    FARPROC p = GetProcAddress(module, name); \
    if (!p) return -1001; \
    memcpy(&e->field, &p, sizeof(p)); \
} while (0)
#define CHECK(call) do { int status = (int)(call); if (status != 0) return status < 0 ? status : -1002; } while (0)
static int initialize(EyeAr087* e, HMODULE ar, HMODULE cv, HMODULE cuda, const char* models) {
    unsigned version = 0;
    RESOLVE(version, ar, "NvAR_GetVersion");
    RESOLVE(create, ar, "NvAR_Create"); RESOLVE(load, ar, "NvAR_Load");
    RESOLVE(run, ar, "NvAR_Run"); RESOLVE(destroy, ar, "NvAR_Destroy");
    RESOLVE(stream_create, ar, "NvAR_CudaStreamCreate"); RESOLVE(stream_destroy, ar, "NvAR_CudaStreamDestroy");
    RESOLVE(u32, ar, "NvAR_SetU32"); RESOLVE(s32, ar, "NvAR_SetS32");
    RESOLVE(object, ar, "NvAR_SetObject"); RESOLVE(string, ar, "NvAR_SetString");
    RESOLVE(stream_set, ar, "NvAR_SetCudaStream"); RESOLVE(array, ar, "NvAR_SetF32Array");
    RESOLVE(alloc, cv, "NvCVImage_Alloc"); RESOLVE(init, cv, "NvCVImage_Init");
    RESOLVE(transfer, cv, "NvCVImage_Transfer"); RESOLVE(dealloc, cv, "NvCVImage_Dealloc");
    RESOLVE(sync, cuda, "cudaStreamSynchronize");
    CHECK(e->version(&version));
    if (version != 0x00080700) return -1003;
    CHECK(e->stream_create(&e->stream));
    CHECK(e->create(NvAR_Feature_GazeRedirection, &e->feature));
    CHECK(e->string(e->feature, NvAR_Parameter_Config(ModelDir), models));
    CHECK(e->u32(e->feature, NvAR_Parameter_Config(Landmarks_Size), 68));
    CHECK(e->u32(e->feature, NvAR_Parameter_Config(Temporal), 0xffffffffu));
    CHECK(e->u32(e->feature, NvAR_Parameter_Config(GazeRedirect), 1));
    CHECK(e->u32(e->feature, NvAR_Parameter_Config(UseCudaGraph), 1));
    CHECK(e->u32(e->feature, NvAR_Parameter_Config(EyeSizeSensitivity), 3));
    CHECK(e->stream_set(e->feature, NvAR_Parameter_Config(CUDAStream), e->stream));
    CHECK(e->load(e->feature));
    CHECK(e->alloc(&e->input, e->width, e->height, NVCV_BGR, NVCV_U8, NVCV_CHUNKY, NVCV_GPU, 1));
    CHECK(e->alloc(&e->output, e->width, e->height, NVCV_BGR, NVCV_U8, NVCV_CHUNKY, NVCV_GPU, 1));
    CHECK(e->object(e->feature, NvAR_Parameter_Input(Image), &e->input, sizeof(e->input)));
    CHECK(e->object(e->feature, NvAR_Parameter_Output(Image), &e->output, sizeof(e->output)));
    CHECK(e->s32(e->feature, NvAR_Parameter_Input(Width), (int)e->width));
    CHECK(e->s32(e->feature, NvAR_Parameter_Input(Height), (int)e->height));
    CHECK(e->object(e->feature, NvAR_Parameter_Output(Landmarks), e->landmarks, sizeof(NvAR_Point2f)));
    CHECK(e->object(e->feature, NvAR_Parameter_Output(GazeOutputLandmarks), e->gaze_landmarks, sizeof(NvAR_Point2f)));
    CHECK(e->array(e->feature, NvAR_Parameter_Output(LandmarksConfidence), e->confidence, 68));
    CHECK(e->array(e->feature, NvAR_Parameter_Output(OutputGazeVector), e->gaze_angles, 2));
    CHECK(e->array(e->feature, NvAR_Parameter_Output(OutputHeadTranslation), e->translation, 3));
    CHECK(e->object(e->feature, NvAR_Parameter_Output(HeadPose), &e->pose, sizeof(e->pose)));
    CHECK(e->object(e->feature, NvAR_Parameter_Output(GazeDirection), e->direction, sizeof(NvAR_Point3f)));
    e->boxes.boxes = &e->box; e->boxes.max_boxes = 1; e->boxes.num_boxes = 0;
    CHECK(e->object(e->feature, NvAR_Parameter_Output(BoundingBoxes), &e->boxes, sizeof(e->boxes)));
    return 0;
}
int eye_ar087_open(HMODULE ar, HMODULE cv, HMODULE cuda, const char* models,
    unsigned width, unsigned height, EyeAr087** out) {
    EyeAr087* e;
    if (!out || !ar || !cv || !cuda || !models || !width || !height || width > 1920 || height > 1080) return -1004;
    *out = 0;
    e = (EyeAr087*)calloc(1, sizeof(*e));
    if (!e) return -1005;
    *out = e; e->width = width; e->height = height;
    return initialize(e, ar, cv, cuda, models); // caller owns partial cleanup too
}
int eye_ar087_process(EyeAr087* e, const unsigned char* input, unsigned char* output) {
    NvCVImage cpu_in = {0}, cpu_out = {0};
    float average = 0;
    unsigned i;
    if (!e || !input || !output || !e->feature) return -1004;
    memset(e->confidence, 0, sizeof(e->confidence));
    memset(&e->box, 0, sizeof(e->box)); e->boxes.num_boxes = 0;
    CHECK(e->init(&cpu_in, e->width, e->height, (int)e->width * 3, (void*)input, NVCV_BGR, NVCV_U8, NVCV_CHUNKY, NVCV_CPU));
    CHECK(e->transfer(&cpu_in, &e->input, 1.f, e->stream, &e->temporary));
    CHECK(e->run(e->feature));
    CHECK(e->sync(e->stream)); // host metadata and GPU completion are observable
    for (i = 0; i < 68; ++i) {
        if (!isfinite(e->confidence[i])) return 1;
        average += e->confidence[i];
    }
    if (e->boxes.num_boxes != 1 || !isfinite(e->box.width) || !isfinite(e->box.height)
        || e->box.width <= 0 || e->box.height <= 0 || average / 68.f < .15f) return 1;
    CHECK(e->init(&cpu_out, e->width, e->height, (int)e->width * 3, output, NVCV_BGR, NVCV_U8, NVCV_CHUNKY, NVCV_CPU));
    CHECK(e->transfer(&e->output, &cpu_out, 1.f, e->stream, &e->temporary));
    CHECK(e->sync(e->stream));
    return 0;
}
int eye_ar087_close(EyeAr087* e) {
    if (!e) return 0;
    // If GPU ownership is uncertain, retain buffers/handles until the supervisor
    // tears down the process. Do not free memory a failed operation may still use.
    if (e->stream && e->sync && e->sync(e->stream)) return -1006;
    if (e->feature && e->destroy && e->destroy(e->feature)) return -1006;
    if (e->dealloc) { e->dealloc(&e->temporary); e->dealloc(&e->output); e->dealloc(&e->input); }
    if (e->stream && e->stream_destroy && e->stream_destroy(e->stream)) return -1006;
    free(e);
    return 0;
}
