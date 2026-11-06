#pragma once
#include "provider.h"
#include <memory>
namespace tingxian::eye {
// Experimental offline worker only. No webview-controlled paths and no SDK
// imports in the base application. Pins are a local, explicit build input.
std::unique_ptr<EyeCorrectionProvider> make_ar087_provider();
}
