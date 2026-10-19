#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <mmdeviceapi.h>
#include <audioclient.h>
#include <functiondiscoverykeys_devpkey.h>
#include <ksmedia.h>
#include <mmsystem.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <string>

struct PacketHeader {
    uint64_t qpc100ns;
    uint32_t frames;
    uint32_t flags;
};

struct Capture {
    const char* name;
    FILE* file;
    IMMDevice* device;
    IAudioClient* client;
    IAudioCaptureClient* capture;
    WAVEFORMATEX* format;
    std::string endpointId;
    std::string endpointName;
};

static std::string Utf8(const wchar_t* value) {
    if (!value || !*value) return std::string();
    int bytes = WideCharToMultiByte(CP_UTF8, 0, value, -1, NULL, 0, NULL, NULL);
    if (bytes <= 1) return std::string();
    std::string result((size_t)bytes, '\0');
    WideCharToMultiByte(CP_UTF8, 0, value, -1, &result[0], bytes, NULL, NULL);
    result.resize((size_t)bytes - 1);
    return result;
}

static void Release(Capture* c) {
    if (c->client) c->client->Stop();
    if (c->capture) c->capture->Release();
    if (c->client) c->client->Release();
    if (c->device) c->device->Release();
    if (c->format) CoTaskMemFree(c->format);
    if (c->file) fclose(c->file);
    c->capture = NULL;
    c->client = NULL;
    c->device = NULL;
    c->format = NULL;
    c->file = NULL;
}

static bool Check(HRESULT hr, const char* what) {
    if (SUCCEEDED(hr)) return true;
    fprintf(stderr, "%s failed: HRESULT=0x%08lx\n", what, (unsigned long)hr);
    return false;
}

static bool InitCapture(Capture* c, IMMDeviceEnumerator* enumerator,
                        EDataFlow flow, DWORD flags, const wchar_t* path) {
    HRESULT hr = enumerator->GetDefaultAudioEndpoint(flow, eConsole, &c->device);
    if (!Check(hr, flow == eRender ? "default render endpoint" : "default capture endpoint")) return false;
    LPWSTR endpointId = NULL;
    if (SUCCEEDED(c->device->GetId(&endpointId))) {
        c->endpointId = Utf8(endpointId);
        CoTaskMemFree(endpointId);
    }
    IPropertyStore* properties = NULL;
    if (SUCCEEDED(c->device->OpenPropertyStore(STGM_READ, &properties))) {
        PROPVARIANT value;
        PropVariantInit(&value);
        if (SUCCEEDED(properties->GetValue(PKEY_Device_FriendlyName, &value)) && value.vt == VT_LPWSTR) {
            c->endpointName = Utf8(value.pwszVal);
        }
        PropVariantClear(&value);
        properties->Release();
    }
    printf("%s endpoint: %s [%s]\n", c->name,
           c->endpointName.empty() ? "(name unavailable)" : c->endpointName.c_str(),
           c->endpointId.c_str());
    hr = c->device->Activate(__uuidof(IAudioClient), CLSCTX_ALL, NULL, (void**)&c->client);
    if (!Check(hr, "activate IAudioClient")) return false;
    hr = c->client->GetMixFormat(&c->format);
    if (!Check(hr, "GetMixFormat")) return false;
    hr = c->client->Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 1000000, 0, c->format, NULL);
    if (!Check(hr, "IAudioClient::Initialize")) return false;
    hr = c->client->GetService(__uuidof(IAudioCaptureClient), (void**)&c->capture);
    if (!Check(hr, "GetService(IAudioCaptureClient)")) return false;
    c->file = _wfopen(path, L"wb");
    if (!c->file) {
        fprintf(stderr, "cannot create capture stream\n");
        return false;
    }
    return true;
}

static void DumpFormat(FILE* meta, const Capture* c) {
    const WAVEFORMATEX* f = c->format;
    fprintf(meta, "%s.endpointName=%s\n", c->name, c->endpointName.c_str());
    fprintf(meta, "%s.endpointId=%s\n", c->name, c->endpointId.c_str());
    fprintf(meta, "%s.formatTag=%u\n", c->name, f->wFormatTag);
    fprintf(meta, "%s.sampleRate=%lu\n", c->name, (unsigned long)f->nSamplesPerSec);
    fprintf(meta, "%s.channels=%u\n", c->name, f->nChannels);
    fprintf(meta, "%s.bitsPerSample=%u\n", c->name, f->wBitsPerSample);
    fprintf(meta, "%s.blockAlign=%u\n", c->name, f->nBlockAlign);
    fprintf(meta, "%s.extraBytes=%u\n", c->name, f->cbSize);
    if (f->wFormatTag == WAVE_FORMAT_EXTENSIBLE && f->cbSize >= 22) {
        const WAVEFORMATEXTENSIBLE* ext = (const WAVEFORMATEXTENSIBLE*)f;
        fprintf(meta, "%s.validBits=%u\n", c->name, ext->Samples.wValidBitsPerSample);
        fprintf(meta, "%s.subFormat=%08lx-%04x-%04x\n", c->name,
                (unsigned long)ext->SubFormat.Data1,
                ext->SubFormat.Data2, ext->SubFormat.Data3);
        fprintf(meta, "%s.channelMask=0x%08lx\n", c->name, (unsigned long)ext->dwChannelMask);
    }
}

static bool Drain(Capture* c) {
    UINT32 packetFrames = 0;
    HRESULT hr = c->capture->GetNextPacketSize(&packetFrames);
    if (!Check(hr, "GetNextPacketSize")) return false;
    while (packetFrames != 0) {
        BYTE* data = NULL;
        UINT32 frames = 0;
        DWORD flags = 0;
        UINT64 devicePosition = 0;
        UINT64 qpcPosition = 0;
        hr = c->capture->GetBuffer(&data, &frames, &flags, &devicePosition, &qpcPosition);
        if (!Check(hr, "GetBuffer")) return false;
        PacketHeader header{qpcPosition, frames, flags};
        bool ok = fwrite(&header, sizeof(header), 1, c->file) == 1;
        size_t bytes = (size_t)frames * c->format->nBlockAlign;
        if (ok && (flags & AUDCLNT_BUFFERFLAGS_SILENT)) {
            BYTE* zeroes = (BYTE*)calloc(bytes ? bytes : 1, 1);
            ok = zeroes && fwrite(zeroes, 1, bytes, c->file) == bytes;
            free(zeroes);
        } else if (ok && bytes != 0) {
            ok = data && fwrite(data, 1, bytes, c->file) == bytes;
        }
        HRESULT releaseHr = c->capture->ReleaseBuffer(frames);
        if (!ok || !Check(releaseHr, "ReleaseBuffer")) {
            fprintf(stderr, "packet write failed for %s\n", c->name);
            return false;
        }
        hr = c->capture->GetNextPacketSize(&packetFrames);
        if (!Check(hr, "GetNextPacketSize")) return false;
    }
    return true;
}

static void Put16(FILE* f, uint16_t v) { fwrite(&v, sizeof(v), 1, f); }
static void Put32(FILE* f, uint32_t v) { fwrite(&v, sizeof(v), 1, f); }

static bool WriteStimulus(const wchar_t* path, bool formal) {
    const uint32_t rate = 48000;
    const uint16_t channels = 2;
    const uint16_t bits = 16;
    const double duration = formal ? 24.0 : 3.0;
    const uint32_t frames = (uint32_t)(duration * rate);
    const uint32_t dataBytes = frames * channels * bits / 8;
    FILE* f = _wfopen(path, L"wb");
    if (!f) return false;
    fwrite("RIFF", 1, 4, f); Put32(f, 36 + dataBytes); fwrite("WAVEfmt ", 1, 8, f);
    Put32(f, 16); Put16(f, WAVE_FORMAT_PCM); Put16(f, channels); Put32(f, rate);
    Put32(f, rate * channels * bits / 8); Put16(f, channels * bits / 8); Put16(f, bits);
    fwrite("data", 1, 4, f); Put32(f, dataBytes);
    const uint32_t chirpFrames = rate / 10;
    const double amplitude = 0.05; // -26 dBFS peak
    const int pulseCount = formal ? 10 : 2;
    const double firstPulse = formal ? 0.8 : 0.45;
    const double interval = formal ? 2.2 : 1.35;
    const double pi = 3.14159265358979323846;
    for (uint32_t i = 0; i < frames; ++i) {
        double value = 0.0;
        for (int p = 0; p < pulseCount; ++p) {
            int64_t start = (int64_t)((firstPulse + interval * p) * rate);
            int64_t local = (int64_t)i - start;
            if (local < 0 || (uint32_t)local >= chirpFrames) continue;
            double t = (double)local / rate;
            double seconds = (double)chirpFrames / rate;
            double phase = 2.0 * pi * (700.0 * t + (8500.0 - 700.0) * t * t / (2.0 * seconds));
            double window = 0.5 - 0.5 * cos(2.0 * pi * t / seconds);
            value = amplitude * window * sin(phase);
            break;
        }
        int16_t sample = (int16_t)(value * 32767.0);
        Put16(f, (uint16_t)sample); Put16(f, (uint16_t)sample);
    }
    fclose(f);
    return true;
}

int wmain(int argc, wchar_t** argv) {
    bool formal = argc > 1 && wcscmp(argv[1], L"formal") == 0;
    const wchar_t* outDir = argc > 2 ? argv[2] : L".";
    wchar_t loopPath[32768], micPath[32768], metaPath[32768], wavPath[32768], cancelPath[32768];
    if (swprintf_s(loopPath, L"%ls\\loopback.pkt", outDir) < 0 ||
        swprintf_s(micPath, L"%ls\\microphone.pkt", outDir) < 0 ||
        swprintf_s(metaPath, L"%ls\\metadata.txt", outDir) < 0 ||
        swprintf_s(wavPath, L"%ls\\stimulus.wav", outDir) < 0 ||
        swprintf_s(cancelPath, L"%ls\\cancel.signal", outDir) < 0) {
        fprintf(stderr, "capture path is too long\n"); return 2;
    }

    if (FAILED(CoInitializeEx(NULL, COINIT_MULTITHREADED))) {
        fprintf(stderr, "CoInitializeEx failed\n"); return 2;
    }
    if (!WriteStimulus(wavPath, formal)) { fprintf(stderr, "stimulus write failed\n"); CoUninitialize(); return 2; }

    IMMDeviceEnumerator* enumerator = NULL;
    HRESULT hr = CoCreateInstance(__uuidof(MMDeviceEnumerator), NULL, CLSCTX_ALL,
                                  __uuidof(IMMDeviceEnumerator), (void**)&enumerator);
    if (!Check(hr, "CoCreateInstance(MMDeviceEnumerator)")) { CoUninitialize(); return 2; }

    Capture loop{}; loop.name = "loopback";
    Capture mic{}; mic.name = "microphone";
    bool ok = InitCapture(&loop, enumerator, eRender, AUDCLNT_STREAMFLAGS_LOOPBACK, loopPath)
           && InitCapture(&mic, enumerator, eCapture, 0, micPath);
    enumerator->Release();
    if (!ok) { Release(&mic); Release(&loop); CoUninitialize(); return 2; }

    FILE* meta = _wfopen(metaPath, L"wb");
    if (!meta) { fprintf(stderr, "metadata write failed\n"); Release(&mic); Release(&loop); CoUninitialize(); return 2; }
    fprintf(meta, "format=AnPod acoustic latency probe v1\nmode=%s\nqpcUnits=100ns\n",
            formal ? "formal" : "precheck");
    fprintf(meta, "packetHeader=uint64 qpc100ns, uint32 frames, uint32 flags; followed by frames*blockAlign PCM bytes\n");
    DumpFormat(meta, &loop); DumpFormat(meta, &mic); fclose(meta);

    if (!Check(loop.client->Start(), "start loopback") || !Check(mic.client->Start(), "start microphone")) {
        Release(&mic); Release(&loop); CoUninitialize(); return 2;
    }
    Sleep(300);
    BOOL played = PlaySoundW(wavPath, NULL, SND_FILENAME | SND_ASYNC | SND_NODEFAULT);
    printf("mode=%s signal=%s amplitude=-26dBFS duration=%.1fs\n",
           formal ? "formal" : "precheck", played ? "started" : "failed", formal ? 24.0 : 3.0);
    if (!played) { Release(&mic); Release(&loop); CoUninitialize(); return 2; }

    ULONGLONG deadline = GetTickCount64() + (formal ? 25500 : 4500);
    bool cancelled = false;
    while (GetTickCount64() < deadline) {
        if (GetFileAttributesW(cancelPath) != INVALID_FILE_ATTRIBUTES) { cancelled = true; break; }
        if (!Drain(&loop) || !Drain(&mic)) {
            PlaySoundW(NULL, NULL, 0); Release(&mic); Release(&loop); CoUninitialize(); return 2;
        }
        Sleep(1);
    }
    PlaySoundW(NULL, NULL, 0);
    fflush(loop.file); fflush(mic.file);
    printf(cancelled ? "capture cancelled; partial data retained locally\n" : "capture complete\n");
    Release(&mic); Release(&loop); CoUninitialize();
    return cancelled ? 3 : 0;
}
