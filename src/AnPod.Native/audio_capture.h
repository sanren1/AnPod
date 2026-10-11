/*
 * AnPod native audio capture — public C ABI.
 *
 * Derived from ysbing/AudioShare client/native/audio_capture.cpp
 * (LGPL-3.0-or-later). See upstream/SOURCE_VERSIONS.md and PATCHES.md.
 *
 * This header is the single place that declares the exported surface; the C#
 * side binds to it and must not hand-write P/Invoke signatures anywhere else.
 *
 * Calling convention: __cdecl. On Windows x64 there is only one convention, so
 * this is documentation rather than a behavioural difference, but it keeps the
 * contract explicit if an x86 build is ever produced.
 */
#ifndef ANPOD_AUDIO_CAPTURE_H
#define ANPOD_AUDIO_CAPTURE_H

#include <stdint.h>

#if defined(_WIN32)
#  define ANPOD_API __declspec(dllexport)
#  define ANPOD_CALL __cdecl
#else
#  define ANPOD_API
#  define ANPOD_CALL
#endif

/* ---- error codes reported by AudioCapture_GetLastErrorCode --------------- */
#define ANPOD_OK                        0

/* Initialisation (AudioCapture_Initialize) */
#define ANPOD_ERR_COM_INIT              1001
#define ANPOD_ERR_ENUMERATOR            1002
#define ANPOD_ERR_NO_ENDPOINT           1003  /* no default render device      */
#define ANPOD_ERR_ACTIVATE              1004
#define ANPOD_ERR_GET_MIX_FORMAT        1005
#define ANPOD_ERR_UNSUPPORTED_FORMAT    1006  /* reject rather than play noise */
#define ANPOD_ERR_AUDIO_CLIENT_INIT     1007
#define ANPOD_ERR_GET_CAPTURE_CLIENT    1008
#define ANPOD_ERR_BUFFER_SIZE           1009
#define ANPOD_ERR_OUT_OF_MEMORY         1010

/* Listener (AudioCapture_Listen) */
#define ANPOD_ERR_WSA_STARTUP           1100
#define ANPOD_ERR_SOCKET_BIND           1101  /* port busy / not permitted     */
#define ANPOD_ERR_SOCKET_LISTEN         1102

/* Handshake */
#define ANPOD_ERR_TOKEN_MISMATCH        1200  /* connect code did not match    */
#define ANPOD_ERR_HANDSHAKE_TIMEOUT     1201

/* Streaming */
#define ANPOD_ERR_DEVICE_INVALIDATED    1300  /* endpoint changed / removed    */
#define ANPOD_ERR_SEND_FAILED           1301

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Handshake callback. Invoked on a native listener thread with the connect
 * code sent by the Android side. The pointer is only valid for the duration of
 * the call; the callee must copy it immediately and must not block.
 */
typedef void (ANPOD_CALL *ConnectCallback)(const char* connectCode);

/*
 * Initialises WASAPI loopback capture on the current default render endpoint.
 * Returns 1 on success, 0 on failure. On failure the reason is available via
 * AudioCapture_GetLastErrorCode.
 *
 * Must be called before AudioCapture_Listen. Not re-entrant: call Cleanup
 * before initialising again.
 */
ANPOD_API int ANPOD_CALL AudioCapture_Initialize(void);

/*
 * Binds a loopback-only TCP listener on 127.0.0.1:port and waits for the
 * Android client. Returns 1 once the socket is listening, 0 otherwise.
 *
 * The listener accepts only a client whose connect code equals the value last
 * passed to AudioCapture_SetConnectToken. A mismatching client is dropped and
 * the listener keeps waiting.
 */
ANPOD_API int ANPOD_CALL AudioCapture_Listen(int port, ConnectCallback callback);

/* Starts the capture thread. Call after the callback has fired. 1 = ok. */
ANPOD_API int ANPOD_CALL AudioCapture_Start(void);

/*
 * Selects what the capture thread forwards: 1 = the captured audio (the
 * default), 0 = silence.
 *
 * Why silence rather than simply stopping: the socket is the session. Closing
 * it would make the Android payload's read() return -1, which exits the
 * process, which would force a full reconnect just to mute and unmute. Sending
 * silence keeps the peer, its AudioTrack and the handshake alive, so pausing
 * and resuming cost nothing and the stream resumes without a re-handshake.
 *
 * The WASAPI client keeps running while disabled - the buffer is still drained
 * and released - so the audio engine never stalls and resuming is seamless.
 * Safe to call at any time after AudioCapture_Start; a no-op before it.
 */
ANPOD_API void ANPOD_CALL AudioCapture_SetStreamingEnabled(int enabled);

/*
 * Signals the capture and listen threads to stop. Returns immediately; it does
 * not join the threads (the caller may be a UI thread).
 */
ANPOD_API void ANPOD_CALL AudioCapture_Stop(void);

/*
 * Stops and joins all threads, closes sockets and releases COM objects.
 * Safe to call repeatedly; after it returns the module is back to its
 * pre-Initialize state.
 */
ANPOD_API void ANPOD_CALL AudioCapture_Cleanup(void);

/*
 * If a client has completed the handshake, copies its connect code into outBuf
 * (NUL-terminated) and returns 1. Returns 0 when no client is connected.
 */
ANPOD_API int ANPOD_CALL AudioCapture_IsConnected(char* outBuf, int bufSize);

/* ---- additive diagnostics (not present in the unpatched upstream DLL) ---- */

/*
 * Sets the connect code the next Listen will require. Pass an empty string or
 * NULL to accept any client (the upstream behaviour).
 */
ANPOD_API void ANPOD_CALL AudioCapture_SetConnectToken(const char* token);

/* Pending error code, or ANPOD_OK when there is none. */
ANPOD_API int ANPOD_CALL AudioCapture_GetLastErrorCode(void);

/*
 * Static NUL-terminated description of the pending error. Owned by the module;
 * do not free. Never returns NULL.
 */
ANPOD_API const char* ANPOD_CALL AudioCapture_GetLastErrorMessage(void);

/* Clears the pending error. */
ANPOD_API void ANPOD_CALL AudioCapture_ClearLastError(void);

/* Total PCM payload bytes accepted by successful TCP send calls this session. */
ANPOD_API uint64_t ANPOD_CALL AudioCapture_GetSentPcmBytes(void);

#ifdef __cplusplus
}
#endif

#endif /* ANPOD_AUDIO_CAPTURE_H */
