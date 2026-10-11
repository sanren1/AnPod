/*
 * AnPod native audio capture.
 *
 * Derived from ysbing/AudioShare client/native/audio_capture.cpp, licensed
 * LGPL-3.0-or-later. The original algorithm (WASAPI loopback -> stereo PCM16
 * -> blocking TCP) is preserved. The changes applied here are listed in
 * upstream/PATCHES.md and are limited to correctness and safety:
 *
 *   1. Listener binds to 127.0.0.1 instead of INADDR_ANY.
 *   2. Handshake connect code is verified against an expected token.
 *   3. Format fallback validates the sample subtype/bit depth instead of
 *      reinterpreting unknown data as float32 (which produced noise).
 *   4. Sends carry a timeout; an incomplete packet ends the session so the
 *      unframed PCM stream can only resume after a fresh handshake.
 *   5. Endpoint invalidation is reported so the caller can rebuild the session.
 *   6. Diagnostics exports, per-session PCM byte counts and balanced
 *      timeBeginPeriod/CoUninitialize.
 *
 * Everything else - including the 1 ms polling loop - is intentionally left as
 * upstream, per plan §3.4.7.
 */

/* Windows 7 is the floor for IAudioClient/SRWLOCK; without this MinGW defaults
   to an XP target and the WASAPI declarations are hidden. */
#ifndef _WIN32_WINNT
#define _WIN32_WINNT 0x0601
#endif

#define WIN32_LEAN_AND_MEAN
#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#include <mmdeviceapi.h>
#include <audioclient.h>
#include <mmreg.h>
#include <mmsystem.h>
#include <string.h>
#include <stdio.h>
#include <stdint.h>
#include <mutex>

#include "audio_capture.h"
#include "send_exact.h"

#pragma comment(lib, "ws2_32.lib")
#pragma comment(lib, "winmm.lib")
#pragma comment(lib, "ole32.lib")

/*
 * KSDATAFORMAT_SUBTYPE_* normally comes from <ksmedia.h>, which drags in a lot
 * of unrelated media headers. The two GUIDs we need are stable ABI, so they are
 * declared locally instead.
 */
static const GUID kSubtypePcm =
    { 0x00000001, 0x0000, 0x0010, { 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71 } };
static const GUID kSubtypeIeeeFloat =
    { 0x00000003, 0x0000, 0x0010, { 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71 } };

/* SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT, likewise avoiding ksmedia.h. */
static const DWORD kStereoChannelMask = 0x1 | 0x2;

/* ------------------------------------------------------------------ state */

static IMMDeviceEnumerator* g_pEnumerator = NULL;
static IAudioClient* g_pAudioClient = NULL;
static IAudioCaptureClient* g_pCaptureClient = NULL;
static HANDLE g_hCaptureThread = NULL;
static HANDLE g_hListenThread = NULL;
static volatile BOOL g_bRunning = FALSE;
static volatile BOOL g_bListening = FALSE;
static volatile BOOL g_bConnected = FALSE;
static volatile LONG64 g_sentPcmBytes = 0;
static std::mutex g_pcmCallbackMutex;
static PcmCallback g_pcmCallback = NULL;
static void* g_pcmCallbackUserData = NULL;

/* What the capture thread forwards. FALSE keeps the socket alive but feeds the
   peer silence, so a session can be muted without being torn down. */
static volatile BOOL g_bStreamingEnabled = TRUE;

static BOOL g_bComInitialized = FALSE;
static BOOL g_bWsaInitialized = FALSE;

/* Source format actually negotiated with WASAPI. */
static int g_nSourceChannels = 2;
static BOOL g_bSourceIsPcm16 = FALSE;   /* FALSE => 32-bit float */
static UINT32 g_nSampleRate = 48000;

/* Format reported to the Android side. Always stereo PCM16. */
static const UINT16 kOutputChannels = 2;
static const UINT16 kOutputBitsPerSample = 16;

static SOCKET g_listenSocket = INVALID_SOCKET;
static SOCKET g_clientSocket = INVALID_SOCKET;
static WSADATA g_wsaData;
static HANDLE g_hListenReady = NULL;
static char g_connectCodeBuf[256] = { 0 };
static char g_expectedToken[256] = { 0 };
static ConnectCallback g_connectCallback = NULL;

/* Short consumer backlog: ~170 ms of 48 kHz stereo PCM16. */
static const int kSocketAudioBufferBytes = 32 * 1024;

/* A send beyond this budget ends the session; the next stream starts fresh. */
static const DWORD kSendTimeoutMs = 200;


/* Error reporting ---------------------------------------------------------- */

static SRWLOCK g_errorLock = SRWLOCK_INIT;
static int g_lastErrorCode = ANPOD_OK;
static char g_lastErrorMessage[256] = { 0 };

/* Bounded copy. strncpy_s is MSVC-only, and the module must also build under
   MinGW for local verification, so the truncation is spelled out here. */
static void CopyString(char* destination, size_t capacity, const char* source)
{
    if (capacity == 0)
    {
        return;
    }

    if (source == NULL)
    {
        destination[0] = '\0';
        return;
    }

    strncpy(destination, source, capacity - 1);
    destination[capacity - 1] = '\0';
}

static void SetLastErrorState(int code, const char* message)
{
    AcquireSRWLockExclusive(&g_errorLock);
    g_lastErrorCode = code;
    CopyString(g_lastErrorMessage, sizeof(g_lastErrorMessage), message);
    ReleaseSRWLockExclusive(&g_errorLock);
}

static const char* DefaultMessageForCode(int code)
{
    switch (code)
    {
        case ANPOD_OK:                    return "";
        case ANPOD_ERR_COM_INIT:          return "COM could not be initialised";
        case ANPOD_ERR_ENUMERATOR:        return "audio device enumerator unavailable";
        case ANPOD_ERR_NO_ENDPOINT:       return "no default Windows audio output device";
        case ANPOD_ERR_ACTIVATE:          return "audio endpoint could not be activated";
        case ANPOD_ERR_GET_MIX_FORMAT:    return "audio endpoint mix format unavailable";
        case ANPOD_ERR_UNSUPPORTED_FORMAT:return "Windows audio format is not supported";
        case ANPOD_ERR_AUDIO_CLIENT_INIT: return "WASAPI loopback could not be initialised";
        case ANPOD_ERR_GET_CAPTURE_CLIENT:return "WASAPI capture interface unavailable";
        case ANPOD_ERR_BUFFER_SIZE:       return "WASAPI buffer size unavailable";
        case ANPOD_ERR_OUT_OF_MEMORY:     return "out of memory";
        case ANPOD_ERR_WSA_STARTUP:       return "Winsock could not be initialised";
        case ANPOD_ERR_SOCKET_BIND:       return "loopback port could not be bound";
        case ANPOD_ERR_SOCKET_LISTEN:     return "loopback socket could not listen";
        case ANPOD_ERR_TOKEN_MISMATCH:    return "connect code did not match this session";
        case ANPOD_ERR_HANDSHAKE_TIMEOUT: return "handshake timed out";
        case ANPOD_ERR_DEVICE_INVALIDATED:return "Windows audio output device changed";
        case ANPOD_ERR_SEND_FAILED:       return "audio socket send failed";
        case ANPOD_ERR_PCM_CALLBACK:      return "PCM sink callback failed";
        default:                          return "unknown audio capture error";
    }
}

/* -------------------------------------------------------------- socket I/O */

/* A short or timed-out send breaks the unframed PCM byte stream; fail the session. */
static int SendPacket(SOCKET sock, const char* data, int len, BOOL countPcm = FALSE)
{
    if (sock == INVALID_SOCKET) return -1;
    return SendExactBytes(len, [sock, data](int offset, int remaining) {
        const int sent = send(sock, data + offset, remaining, 0);
        if (sent == SOCKET_ERROR && WSAGetLastError() == WSAETIMEDOUT) return 0;
        return sent;
    }, [countPcm](int sent) {
        if (countPcm) InterlockedAdd64(&g_sentPcmBytes, sent);
    });
}

static BOOL SendFormatHeader(SOCKET sock)
{
    BYTE header[8];
    UINT32 sampleRate = g_nSampleRate;
    UINT16 channels = kOutputChannels;
    UINT16 bits = kOutputBitsPerSample;

    memcpy(header + 0, &sampleRate, 4);
    memcpy(header + 4, &channels, 2);
    memcpy(header + 6, &bits, 2);

    return SendPacket(sock, (const char*)header, 8) == (int)sizeof(header);
}

/* ------------------------------------------------------------ listen thread */

static DWORD WINAPI ListenThread(LPVOID lpParam)
{
    int port = (int)(intptr_t)lpParam;

    g_listenSocket = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if (g_listenSocket == INVALID_SOCKET)
    {
        SetLastErrorState(ANPOD_ERR_SOCKET_BIND, DefaultMessageForCode(ANPOD_ERR_SOCKET_BIND));
        if (g_hListenReady) SetEvent(g_hListenReady);
        return 1;
    }

    BOOL reuse = TRUE;
    setsockopt(g_listenSocket, SOL_SOCKET, SO_REUSEADDR, (const char*)&reuse, sizeof(reuse));

    sockaddr_in addr = {};
    addr.sin_family = AF_INET;
    addr.sin_port = htons((u_short)port);
    /* Loopback only: the ADB reverse tunnel terminates locally, so nothing
       outside this machine ever needs to reach the port (plan §3.4.3). */
    addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);

    if (bind(g_listenSocket, (sockaddr*)&addr, sizeof(addr)) == SOCKET_ERROR)
    {
        SetLastErrorState(ANPOD_ERR_SOCKET_BIND, DefaultMessageForCode(ANPOD_ERR_SOCKET_BIND));
        closesocket(g_listenSocket);
        g_listenSocket = INVALID_SOCKET;
        if (g_hListenReady) SetEvent(g_hListenReady);
        return 1;
    }

    if (listen(g_listenSocket, 1) == SOCKET_ERROR)
    {
        SetLastErrorState(ANPOD_ERR_SOCKET_LISTEN, DefaultMessageForCode(ANPOD_ERR_SOCKET_LISTEN));
        closesocket(g_listenSocket);
        g_listenSocket = INVALID_SOCKET;
        if (g_hListenReady) SetEvent(g_hListenReady);
        return 1;
    }

    g_bListening = TRUE;
    if (g_hListenReady) SetEvent(g_hListenReady);

    while (g_bListening)
    {
        sockaddr_in clientAddr = {};
        int addrLen = sizeof(clientAddr);
        SOCKET client = accept(g_listenSocket, (sockaddr*)&clientAddr, &addrLen);

        if (client == INVALID_SOCKET)
        {
            /* Listening socket closed by Stop(); normal shutdown path. */
            break;
        }

        int sndbuf = kSocketAudioBufferBytes;
        setsockopt(client, SOL_SOCKET, SO_SNDBUF, (const char*)&sndbuf, sizeof(sndbuf));

        BOOL noDelay = TRUE;
        setsockopt(client, IPPROTO_TCP, TCP_NODELAY, (const char*)&noDelay, sizeof(noDelay));

        /* Never let a silent peer pin this thread forever. */
        DWORD recvTimeout = 5000;
        setsockopt(client, SOL_SOCKET, SO_RCVTIMEO, (const char*)&recvTimeout, sizeof(recvTimeout));

        DWORD sendTimeout = kSendTimeoutMs;
        setsockopt(client, SOL_SOCKET, SO_SNDTIMEO, (const char*)&sendTimeout, sizeof(sendTimeout));

        char buf[256] = {};
        int received = recv(client, buf, sizeof(buf) - 1, 0);

        if (received <= 0)
        {
            closesocket(client);
            continue;
        }

        buf[received] = '\0';

        /* Verify the handshake before accepting the peer (plan §3.4.4).
           An empty expected token preserves the upstream permissive mode. */
        if (g_expectedToken[0] != '\0' && strcmp(buf, g_expectedToken) != 0)
        {
            SetLastErrorState(ANPOD_ERR_TOKEN_MISMATCH, DefaultMessageForCode(ANPOD_ERR_TOKEN_MISMATCH));
            closesocket(client);
            continue;
        }

        g_clientSocket = client;
        memcpy(g_connectCodeBuf, buf, (size_t)received + 1);
        g_bConnected = TRUE;

        if (g_connectCallback != NULL)
        {
            g_connectCallback(g_connectCodeBuf);
        }

        /* One streaming peer at a time (plan §5.5). */
        break;
    }

    return 0;
}

/* ----------------------------------------------------------- capture thread */

/* Converts one WASAPI buffer into interleaved stereo PCM16. */
static void ConvertToStereoPcm16(
    const BYTE* source,
    UINT32 frames,
    short* destination)
{
    const int channels = g_nSourceChannels;

    if (g_bSourceIsPcm16)
    {
        const short* src = (const short*)source;

        if (channels == 2)
        {
            memcpy(destination, src, (size_t)frames * 2 * sizeof(short));
        }
        else if (channels == 1)
        {
            for (UINT32 i = 0; i < frames; ++i)
            {
                destination[i * 2 + 0] = src[i];
                destination[i * 2 + 1] = src[i];
            }
        }
        else
        {
            /* Surround: take the front pair. A proper downmix is out of scope
               for v1 (plan §10) but silence would be worse. */
            for (UINT32 i = 0; i < frames; ++i)
            {
                destination[i * 2 + 0] = src[i * channels + 0];
                destination[i * 2 + 1] = src[i * channels + 1];
            }
        }

        return;
    }

    const float* src = (const float*)source;

    for (UINT32 i = 0; i < frames; ++i)
    {
        float left;
        float right;

        if (channels == 2)
        {
            left = src[i * 2 + 0];
            right = src[i * 2 + 1];
        }
        else if (channels == 1)
        {
            left = right = src[i];
        }
        else
        {
            left = src[i * channels + 0];
            right = src[i * channels + 1];
        }

        if (left > 1.0f) left = 1.0f; else if (left < -1.0f) left = -1.0f;
        if (right > 1.0f) right = 1.0f; else if (right < -1.0f) right = -1.0f;

        destination[i * 2 + 0] = (short)(left * 32767.0f);
        destination[i * 2 + 1] = (short)(right * 32767.0f);
    }
}

static DWORD WINAPI CaptureThread(LPVOID lpParam)
{
    (void)lpParam;
    uint64_t nextFrameIndex = 0;

    /* 1 ms timer resolution so Sleep(1) does not become a 15.6 ms burst that
       underruns the Android AudioTrack. Balanced in the exit path below. */
    timeBeginPeriod(1);

    const int kMaxFrames = 48000; /* one second of stereo PCM16 */
    short* outBuf = (short*)HeapAlloc(GetProcessHeap(), 0, (SIZE_T)kMaxFrames * 2 * sizeof(short));

    if (outBuf == NULL)
    {
        SetLastErrorState(ANPOD_ERR_OUT_OF_MEMORY, "out of memory allocating conversion buffer");
        timeEndPeriod(1);
        return 1;
    }

    PcmCallback pcmCallback = NULL;
    {
        std::lock_guard<std::mutex> lock(g_pcmCallbackMutex);
        pcmCallback = g_pcmCallback;
    }

    /* Format header first only for the legacy TCP sink. */
    if (pcmCallback == NULL && g_clientSocket != INVALID_SOCKET)
    {
        if (!SendFormatHeader(g_clientSocket))
        {
            SetLastErrorState(ANPOD_ERR_SEND_FAILED, "audio format header was not sent completely");
            g_bRunning = FALSE;
            closesocket(g_clientSocket);
            g_clientSocket = INVALID_SOCKET;
            g_bConnected = FALSE;
        }
    }

    while (g_bRunning)
    {
        Sleep(1);

        if (!g_pCaptureClient || !g_bRunning)
        {
            break;
        }

        UINT32 packetLength = 0;
        HRESULT hr = g_pCaptureClient->GetNextPacketSize(&packetLength);

        if (hr == AUDCLNT_E_DEVICE_INVALIDATED)
        {
            SetLastErrorState(ANPOD_ERR_DEVICE_INVALIDATED,
                DefaultMessageForCode(ANPOD_ERR_DEVICE_INVALIDATED));
            break;
        }

        if (FAILED(hr))
        {
            break;
        }

        while (packetLength != 0 && g_bRunning)
        {
            BYTE* pData = NULL;
            UINT32 numFramesAvailable = 0;
            DWORD flags = 0;

            hr = g_pCaptureClient->GetBuffer(&pData, &numFramesAvailable, &flags, NULL, NULL);

            if (hr == AUDCLNT_E_DEVICE_INVALIDATED)
            {
                SetLastErrorState(ANPOD_ERR_DEVICE_INVALIDATED,
                    DefaultMessageForCode(ANPOD_ERR_DEVICE_INVALIDATED));
                g_bRunning = FALSE;
                break;
            }

            if (FAILED(hr))
            {
                break;
            }

            UINT32 frames = numFramesAvailable;
            if (frames > (UINT32)kMaxFrames)
            {
                /* Only the leading second is converted; the rest is released
                   below so the engine is not stalled. */
                frames = (UINT32)kMaxFrames;
            }

            if (!g_bStreamingEnabled)
            {
                /* Paused: the peer must keep receiving frames or its AudioTrack
                   underruns and the resume is audible as a gap. Silence keeps
                   the pipeline primed and the socket alive (plan §5.3). */
                memset(outBuf, 0, (size_t)frames * 2 * sizeof(short));
            }
            else if ((flags & AUDCLNT_BUFFERFLAGS_SILENT) != 0 || pData == NULL)
            {
                memset(outBuf, 0, (size_t)frames * 2 * sizeof(short));
            }
            else
            {
                ConvertToStereoPcm16(pData, frames, outBuf);
            }

            /* Release before sending so the engine can keep filling while we
               are blocked on the socket (avoids DATA_DISCONTINUITY). */
            g_pCaptureClient->ReleaseBuffer(numFramesAvailable);

            if (frames > UINT64_MAX - nextFrameIndex)
            {
                SetLastErrorState(ANPOD_ERR_PCM_CALLBACK, "PCM sample frame index exhausted");
                g_bRunning = FALSE;
                break;
            }

            BOOL forwardedToPcmSink = FALSE;
            {
                std::lock_guard<std::mutex> lock(g_pcmCallbackMutex);
                if (g_pcmCallback != NULL)
                {
                    g_pcmCallback((const int16_t*)outBuf, frames, g_nSampleRate,
                        kOutputChannels, nextFrameIndex, g_pcmCallbackUserData);
                    forwardedToPcmSink = TRUE;
                }
            }
            nextFrameIndex += frames;

            if (!forwardedToPcmSink && g_clientSocket != INVALID_SOCKET)
            {
                int result = SendPacket(
                    g_clientSocket,
                    (const char*)outBuf,
                    (int)(frames * 2 * sizeof(short)),
                    TRUE);

                if (result < 0)
                {
                    SetLastErrorState(ANPOD_ERR_SEND_FAILED,
                        "audio send timed out or ended before the PCM packet was complete");
                    g_bRunning = FALSE;
                    closesocket(g_clientSocket);
                    g_clientSocket = INVALID_SOCKET;
                    g_bConnected = FALSE;
                    break;
                }
            }

            hr = g_pCaptureClient->GetNextPacketSize(&packetLength);

            if (hr == AUDCLNT_E_DEVICE_INVALIDATED)
            {
                SetLastErrorState(ANPOD_ERR_DEVICE_INVALIDATED,
                    DefaultMessageForCode(ANPOD_ERR_DEVICE_INVALIDATED));
                g_bRunning = FALSE;
                break;
            }

            if (FAILED(hr))
            {
                break;
            }
        }
    }

    HeapFree(GetProcessHeap(), 0, outBuf);
    timeEndPeriod(1);
    return 0;
}

/* ------------------------------------------------------------------ exports */

/*
 * Decides whether a negotiated mix format can be converted to stereo PCM16.
 * Returns TRUE and fills the source-format globals on success.
 *
 * The upstream code assumed "not PCM16 => float32" and produced noise for any
 * other format. Unsupported depths are now rejected (plan §3.4.6).
 */
static BOOL AnalyseMixFormat(const WAVEFORMATEX* format)
{
    if (format == NULL || format->nChannels == 0)
    {
        return FALSE;
    }

    BOOL isFloat = FALSE;
    BOOL isPcm = FALSE;

    if (format->wFormatTag == WAVE_FORMAT_IEEE_FLOAT)
    {
        isFloat = TRUE;
    }
    else if (format->wFormatTag == WAVE_FORMAT_PCM)
    {
        isPcm = TRUE;
    }
    else if (format->wFormatTag == WAVE_FORMAT_EXTENSIBLE &&
             (size_t)format->cbSize >= (sizeof(WAVEFORMATEXTENSIBLE) - sizeof(WAVEFORMATEX)))
    {
        const WAVEFORMATEXTENSIBLE* ext = (const WAVEFORMATEXTENSIBLE*)format;

        if (IsEqualGUID(ext->SubFormat, kSubtypeIeeeFloat))
        {
            isFloat = TRUE;
        }
        else if (IsEqualGUID(ext->SubFormat, kSubtypePcm))
        {
            isPcm = TRUE;
        }
    }

    if (isFloat)
    {
        if (format->wBitsPerSample != 32)
        {
            return FALSE;
        }

        g_bSourceIsPcm16 = FALSE;
    }
    else if (isPcm)
    {
        if (format->wBitsPerSample != 16)
        {
            /* 24/32-bit integer PCM would need a real converter. */
            return FALSE;
        }

        /* A 24-in-32 container would silently misalign; reject it. */
        if (format->wFormatTag == WAVE_FORMAT_EXTENSIBLE)
        {
            const WAVEFORMATEXTENSIBLE* ext = (const WAVEFORMATEXTENSIBLE*)format;
            if (ext->Samples.wValidBitsPerSample != 0 &&
                ext->Samples.wValidBitsPerSample != 16)
            {
                return FALSE;
            }
        }

        g_bSourceIsPcm16 = TRUE;
    }
    else
    {
        return FALSE;
    }

    if (format->nSamplesPerSec < 8000 || format->nSamplesPerSec > 192000)
    {
        return FALSE;
    }

    g_nSourceChannels = format->nChannels;
    g_nSampleRate = format->nSamplesPerSec;
    return TRUE;
}

static void ResetSessionState()
{
    g_bConnected = FALSE;
    g_bStreamingEnabled = TRUE;
    g_connectCodeBuf[0] = '\0';
    g_nSourceChannels = 2;
    g_bSourceIsPcm16 = FALSE;
    g_nSampleRate = 48000;
}

int ANPOD_CALL AudioCapture_Initialize(void)
{
    HRESULT hr;

    if (g_pAudioClient != NULL)
    {
        /* Already initialised; treat as success (callers may retry). */
        return 1;
    }

    ResetSessionState();

    hr = CoInitializeEx(NULL, COINIT_MULTITHREADED);
    if (hr == RPC_E_CHANGED_MODE)
    {
        /* Another apartment already initialised this thread; we must not
           balance it with CoUninitialize. */
        g_bComInitialized = FALSE;
    }
    else if (FAILED(hr))
    {
        SetLastErrorState(ANPOD_ERR_COM_INIT, DefaultMessageForCode(ANPOD_ERR_COM_INIT));
        return 0;
    }
    else
    {
        g_bComInitialized = TRUE;
    }

    hr = CoCreateInstance(
        __uuidof(MMDeviceEnumerator), NULL, CLSCTX_ALL,
        __uuidof(IMMDeviceEnumerator), (void**)&g_pEnumerator);

    if (FAILED(hr) || g_pEnumerator == NULL)
    {
        SetLastErrorState(ANPOD_ERR_ENUMERATOR, DefaultMessageForCode(ANPOD_ERR_ENUMERATOR));
        AudioCapture_Cleanup();
        return 0;
    }

    IMMDevice* pDevice = NULL;
    hr = g_pEnumerator->GetDefaultAudioEndpoint(eRender, eConsole, &pDevice);

    if (FAILED(hr) || pDevice == NULL)
    {
        SetLastErrorState(ANPOD_ERR_NO_ENDPOINT, DefaultMessageForCode(ANPOD_ERR_NO_ENDPOINT));
        AudioCapture_Cleanup();
        return 0;
    }

    hr = pDevice->Activate(__uuidof(IAudioClient), CLSCTX_ALL, NULL, (void**)&g_pAudioClient);

    if (FAILED(hr) || g_pAudioClient == NULL)
    {
        SetLastErrorState(ANPOD_ERR_ACTIVATE, DefaultMessageForCode(ANPOD_ERR_ACTIVATE));
        pDevice->Release();
        AudioCapture_Cleanup();
        return 0;
    }

    WAVEFORMATEX* pwfx = NULL;
    hr = g_pAudioClient->GetMixFormat(&pwfx);

    if (FAILED(hr) || pwfx == NULL)
    {
        SetLastErrorState(ANPOD_ERR_GET_MIX_FORMAT, DefaultMessageForCode(ANPOD_ERR_GET_MIX_FORMAT));
        pDevice->Release();
        AudioCapture_Cleanup();
        return 0;
    }

    /*
     * Preferred path: ask WASAPI for exactly what the protocol needs -
     * 48 kHz stereo PCM16. Shared mode resamples and downmixes for us, which
     * avoids the drift that caused periodic underruns with a 44.1 kHz source.
     */
    WAVEFORMATEXTENSIBLE target = {};
    target.Format.wFormatTag = WAVE_FORMAT_EXTENSIBLE;
    target.Format.nChannels = kOutputChannels;
    target.Format.nSamplesPerSec = 48000;
    target.Format.wBitsPerSample = kOutputBitsPerSample;
    target.Format.nBlockAlign = (WORD)(kOutputChannels * kOutputBitsPerSample / 8);
    target.Format.nAvgBytesPerSec = 48000 * target.Format.nBlockAlign;
    target.Format.cbSize = sizeof(WAVEFORMATEXTENSIBLE) - sizeof(WAVEFORMATEX);
    target.SubFormat = kSubtypePcm;
    target.Samples.wValidBitsPerSample = kOutputBitsPerSample;
    target.dwChannelMask = kStereoChannelMask;

    hr = g_pAudioClient->Initialize(
        AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
        0, 0, (WAVEFORMATEX*)&target, NULL);

    if (SUCCEEDED(hr))
    {
        g_nSourceChannels = kOutputChannels;
        g_bSourceIsPcm16 = TRUE;
        g_nSampleRate = 48000;
    }
    else
    {
        /* Fallback: use the device mix format, but only if it is one we can
           actually convert. */
        if (!AnalyseMixFormat(pwfx))
        {
            SetLastErrorState(
                ANPOD_ERR_UNSUPPORTED_FORMAT,
                DefaultMessageForCode(ANPOD_ERR_UNSUPPORTED_FORMAT));
            CoTaskMemFree(pwfx);
            pDevice->Release();
            AudioCapture_Cleanup();
            return 0;
        }

        hr = g_pAudioClient->Initialize(
            AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
            0, 0, pwfx, NULL);

        if (FAILED(hr))
        {
            SetLastErrorState(
                ANPOD_ERR_AUDIO_CLIENT_INIT,
                DefaultMessageForCode(ANPOD_ERR_AUDIO_CLIENT_INIT));
            CoTaskMemFree(pwfx);
            pDevice->Release();
            AudioCapture_Cleanup();
            return 0;
        }
    }

    CoTaskMemFree(pwfx);
    pDevice->Release();

    UINT32 bufferFrameCount = 0;
    hr = g_pAudioClient->GetBufferSize(&bufferFrameCount);

    if (FAILED(hr))
    {
        SetLastErrorState(ANPOD_ERR_BUFFER_SIZE, DefaultMessageForCode(ANPOD_ERR_BUFFER_SIZE));
        AudioCapture_Cleanup();
        return 0;
    }

    hr = g_pAudioClient->GetService(__uuidof(IAudioCaptureClient), (void**)&g_pCaptureClient);

    if (FAILED(hr) || g_pCaptureClient == NULL)
    {
        SetLastErrorState(
            ANPOD_ERR_GET_CAPTURE_CLIENT,
            DefaultMessageForCode(ANPOD_ERR_GET_CAPTURE_CLIENT));
        AudioCapture_Cleanup();
        return 0;
    }

    SetLastErrorState(ANPOD_OK, NULL);
    return 1;
}

int ANPOD_CALL AudioCapture_Listen(int port, ConnectCallback callback)
{
    {
        std::lock_guard<std::mutex> lock(g_pcmCallbackMutex);
        if (g_pcmCallback != NULL)
        {
            SetLastErrorState(ANPOD_ERR_SOCKET_LISTEN, "TCP listener cannot be used with the PCM sink");
            return 0;
        }
    }

    g_bRunning = FALSE;
    g_connectCallback = callback;
    g_bListening = FALSE;
    g_bConnected = FALSE;
    g_connectCodeBuf[0] = '\0';

    /* Close sockets first so a blocked accept() returns immediately, then join
       the old threads. Closing after the join caused a 2 s stall on every
       reconnect. */
    if (g_clientSocket != INVALID_SOCKET)
    {
        closesocket(g_clientSocket);
        g_clientSocket = INVALID_SOCKET;
    }

    if (g_listenSocket != INVALID_SOCKET)
    {
        closesocket(g_listenSocket);
        g_listenSocket = INVALID_SOCKET;
    }

    if (g_hCaptureThread != NULL)
    {
        WaitForSingleObject(g_hCaptureThread, 1000);
        CloseHandle(g_hCaptureThread);
        g_hCaptureThread = NULL;
    }

    if (g_hListenThread != NULL)
    {
        WaitForSingleObject(g_hListenThread, 2000);
        CloseHandle(g_hListenThread);
        g_hListenThread = NULL;
    }

    InterlockedExchange64(&g_sentPcmBytes, 0);

    if (!g_bWsaInitialized)
    {
        if (WSAStartup(MAKEWORD(2, 2), &g_wsaData) != 0)
        {
            SetLastErrorState(ANPOD_ERR_WSA_STARTUP, DefaultMessageForCode(ANPOD_ERR_WSA_STARTUP));
            return 0;
        }

        g_bWsaInitialized = TRUE;
    }

    if (g_hListenReady != NULL)
    {
        CloseHandle(g_hListenReady);
        g_hListenReady = NULL;
    }

    g_hListenReady = CreateEvent(NULL, TRUE, FALSE, NULL);

    if (g_hListenReady == NULL)
    {
        SetLastErrorState(ANPOD_ERR_SOCKET_LISTEN, "could not create listener event");
        return 0;
    }

    g_hListenThread = CreateThread(NULL, 0, ListenThread, (LPVOID)(intptr_t)port, 0, NULL);

    if (g_hListenThread == NULL)
    {
        CloseHandle(g_hListenReady);
        g_hListenReady = NULL;
        SetLastErrorState(ANPOD_ERR_SOCKET_LISTEN, DefaultMessageForCode(ANPOD_ERR_SOCKET_LISTEN));
        return 0;
    }

    DWORD waitResult = WaitForSingleObject(g_hListenReady, 5000);
    CloseHandle(g_hListenReady);
    g_hListenReady = NULL;

    if (waitResult != WAIT_OBJECT_0 || !g_bListening)
    {
        /* ListenThread already recorded the specific reason. */
        return 0;
    }

    SetLastErrorState(ANPOD_OK, NULL);
    return 1;
}

int ANPOD_CALL AudioCapture_SetPcmCallback(PcmCallback callback, void* userData)
{
    std::lock_guard<std::mutex> lock(g_pcmCallbackMutex);
    if (g_bRunning)
    {
        SetLastErrorState(ANPOD_ERR_PCM_CALLBACK, "PCM sink can only change while capture is stopped");
        return 0;
    }
    if (callback != NULL && (g_bListening || g_bConnected))
    {
        SetLastErrorState(ANPOD_ERR_PCM_CALLBACK, "PCM sink cannot be selected while the TCP listener is active");
        return 0;
    }
    if ((callback == NULL) != (userData == NULL))
    {
        SetLastErrorState(ANPOD_ERR_PCM_CALLBACK, "callback and userData must both be null or non-null");
        return 0;
    }
    g_pcmCallback = callback;
    g_pcmCallbackUserData = userData;
    SetLastErrorState(ANPOD_OK, NULL);
    return 1;
}

int ANPOD_CALL AudioCapture_Start(void)
{
    if (g_pAudioClient == NULL)
    {
        SetLastErrorState(ANPOD_ERR_AUDIO_CLIENT_INIT, "capture was not initialised");
        return 0;
    }

    HRESULT hr = g_pAudioClient->Start();

    if (FAILED(hr))
    {
        SetLastErrorState(ANPOD_ERR_AUDIO_CLIENT_INIT, "WASAPI could not start capture");
        return 0;
    }

    InterlockedExchange64(&g_sentPcmBytes, 0);
    g_bRunning = TRUE;
    g_hCaptureThread = CreateThread(NULL, 0, CaptureThread, NULL, 0, NULL);

    if (g_hCaptureThread == NULL)
    {
        g_bRunning = FALSE;
        g_pAudioClient->Stop();
        SetLastErrorState(ANPOD_ERR_AUDIO_CLIENT_INIT, "capture thread could not be created");
        return 0;
    }

    SetLastErrorState(ANPOD_OK, NULL);
    return 1;
}

void ANPOD_CALL AudioCapture_SetStreamingEnabled(int enabled)
{
    /* A single flag read once per capture iteration; no lock needed. The
       capture thread may be mid-packet, so the change takes effect on the
       next frame, which is inaudible. */
    g_bStreamingEnabled = enabled ? TRUE : FALSE;
}

void ANPOD_CALL AudioCapture_Stop(void)
{
    /* Signal only; never join here. This is called from the UI thread through
       the managed layer and must not block rendering. */
    g_bRunning = FALSE;
    g_bListening = FALSE;

    if (g_clientSocket != INVALID_SOCKET)
    {
        closesocket(g_clientSocket);
        g_clientSocket = INVALID_SOCKET;
    }

    if (g_listenSocket != INVALID_SOCKET)
    {
        closesocket(g_listenSocket);
        g_listenSocket = INVALID_SOCKET;
    }

    if (g_pAudioClient != NULL)
    {
        g_pAudioClient->Stop();
    }
}

void ANPOD_CALL AudioCapture_Cleanup(void)
{
    AudioCapture_Stop();

    if (g_hCaptureThread != NULL)
    {
        WaitForSingleObject(g_hCaptureThread, INFINITE);
        CloseHandle(g_hCaptureThread);
        g_hCaptureThread = NULL;
    }

    if (g_hListenThread != NULL)
    {
        WaitForSingleObject(g_hListenThread, 2000);
        CloseHandle(g_hListenThread);
        g_hListenThread = NULL;
    }

    if (g_hListenReady != NULL)
    {
        CloseHandle(g_hListenReady);
        g_hListenReady = NULL;
    }

    if (g_bWsaInitialized)
    {
        WSACleanup();
        g_bWsaInitialized = FALSE;
    }

    if (g_pCaptureClient != NULL)
    {
        g_pCaptureClient->Release();
        g_pCaptureClient = NULL;
    }

    if (g_pAudioClient != NULL)
    {
        g_pAudioClient->Release();
        g_pAudioClient = NULL;
    }

    if (g_pEnumerator != NULL)
    {
        g_pEnumerator->Release();
        g_pEnumerator = NULL;
    }

    /* Only balance a CoInitializeEx we actually performed (plan §5.5). */
    if (g_bComInitialized)
    {
        CoUninitialize();
        g_bComInitialized = FALSE;
    }

    InterlockedExchange64(&g_sentPcmBytes, 0);

    {
        std::lock_guard<std::mutex> lock(g_pcmCallbackMutex);
        g_pcmCallback = NULL;
        g_pcmCallbackUserData = NULL;
    }

    g_connectCallback = NULL;
    g_bConnected = FALSE;
    g_connectCodeBuf[0] = '\0';
}

int ANPOD_CALL AudioCapture_IsConnected(char* outBuf, int bufSize)
{
    if (!g_bConnected)
    {
        return 0;
    }

    if (outBuf != NULL && bufSize > 0)
    {
        int length = (int)strlen(g_connectCodeBuf);
        if (length >= bufSize)
        {
            length = bufSize - 1;
        }

        memcpy(outBuf, g_connectCodeBuf, (size_t)length);
        outBuf[length] = '\0';
    }

    return 1;
}

uint64_t ANPOD_CALL AudioCapture_GetSentPcmBytes(void)
{
    return (uint64_t)InterlockedCompareExchange64(&g_sentPcmBytes, 0, 0);
}

void ANPOD_CALL AudioCapture_SetConnectToken(const char* token)
{
    if (token == NULL)
    {
        g_expectedToken[0] = '\0';
        return;
    }

    CopyString(g_expectedToken, sizeof(g_expectedToken), token);
}

int ANPOD_CALL AudioCapture_GetLastErrorCode(void)
{
    int code;
    AcquireSRWLockShared(&g_errorLock);
    code = g_lastErrorCode;
    ReleaseSRWLockShared(&g_errorLock);
    return code;
}

const char* ANPOD_CALL AudioCapture_GetLastErrorMessage(void)
{
    /* Static storage: the buffer is only mutated under the lock, and readers
       accept a torn read of a human-readable string. */
    if (g_lastErrorMessage[0] == '\0')
    {
        return DefaultMessageForCode(AudioCapture_GetLastErrorCode());
    }

    return g_lastErrorMessage;
}

void ANPOD_CALL AudioCapture_ClearLastError(void)
{
    SetLastErrorState(ANPOD_OK, NULL);
}
