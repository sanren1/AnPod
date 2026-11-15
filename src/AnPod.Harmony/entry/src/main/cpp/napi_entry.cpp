#include <napi/native_api.h>

#include <string>
#include <vector>

namespace anpod {
std::vector<std::string> runSelfTests();
}

namespace {

napi_value SelfTest(napi_env env, napi_callback_info) {
    std::vector<std::string> failures = anpod::runSelfTests();
    napi_value out;
    napi_create_array_with_length(env, failures.size(), &out);
    for (size_t i = 0; i < failures.size(); i++) {
        napi_value name;
        napi_create_string_utf8(env, failures[i].c_str(), failures[i].size(), &name);
        napi_set_element(env, out, static_cast<uint32_t>(i), name);
    }
    return out;
}

napi_value Init(napi_env env, napi_value exports) {
    napi_property_descriptor desc[] = {
        {"selfTest", nullptr, SelfTest, nullptr, nullptr, nullptr, napi_default, nullptr}};
    napi_define_properties(env, exports, 1, desc);
    return exports;
}

napi_module mod = {1, 0, nullptr, Init, "anpod_core", nullptr, {nullptr}};

}  // namespace

extern "C" __attribute__((constructor)) void RegisterAnpodCore() { napi_module_register(&mod); }
