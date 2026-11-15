// Task 1 throwaway probe: reports which native capabilities the device offers.
#include <napi/native_api.h>

#include <CryptoArchitectureKit/crypto_architecture_kit.h>
#include <asset/asset_api.h>
#include <ohaudio/native_audiorenderer.h>
#include <ohaudio/native_audiostreambuilder.h>

#include <arpa/inet.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <unistd.h>

#include <cerrno>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

namespace {

using Bytes = std::vector<uint8_t>;

std::string hex(const uint8_t* p, size_t n) {
    std::string s;
    char b[3];
    for (size_t i = 0; i < n; i++) { snprintf(b, sizeof b, "%02x", p[i]); s += b; }
    return s;
}

Bytes fromHex(const char* h) {
    Bytes out;
    for (size_t i = 0; h[i] && h[i + 1]; i += 2) {
        unsigned v = 0;
        sscanf(h + i, "%2x", &v);
        out.push_back(static_cast<uint8_t>(v));
    }
    return out;
}

Crypto_DataBlob blob(const Bytes& b) { return Crypto_DataBlob{const_cast<uint8_t*>(b.data()), b.size()}; }

std::string fmt(const char* f, int v) { char b[96]; snprintf(b, sizeof b, f, v); return b; }

std::string probeSha256() {
    OH_CryptoDigest* d = nullptr;
    int rc = OH_CryptoDigest_Create("SHA256", &d);
    if (rc) return "create=" + std::to_string(rc);
    Bytes in = {'a', 'b', 'c'};
    Crypto_DataBlob i = blob(in), o{};
    OH_CryptoDigest_Update(d, &i);
    rc = OH_CryptoDigest_Final(d, &o);
    std::string r = rc ? "final=" + std::to_string(rc) : hex(o.data, o.len);
    OH_Crypto_FreeDataBlob(&o);
    OH_DigestCrypto_Destroy(d);
    return r + (r == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad" ? " MATCH" : " MISMATCH");
}

std::string probeHmac() {
    OH_CryptoSymKeyGenerator* g = nullptr;
    int rc = OH_CryptoSymKeyGenerator_Create("HMAC", &g);
    if (rc) return "keygen=" + std::to_string(rc);
    Bytes key = {'J', 'e', 'f', 'e'};
    Crypto_DataBlob kb = blob(key);
    OH_CryptoSymKey* k = nullptr;
    rc = OH_CryptoSymKeyGenerator_Convert(g, &kb, &k);
    if (rc) { OH_CryptoSymKeyGenerator_Destroy(g); return "convert=" + std::to_string(rc); }
    OH_CryptoMac* m = nullptr;
    rc = OH_CryptoMac_Create("HMAC", &m);
    std::string r;
    if (rc) r = "create=" + std::to_string(rc);
    else {
        Bytes name = {'S', 'H', 'A', '2', '5', '6'};
        Crypto_DataBlob nb = blob(name);
        rc = OH_CryptoMac_SetParam(m, CRYPTO_MAC_DIGEST_NAME_STR, &nb);
        if (!rc) rc = OH_CryptoMac_Init(m, k);
        const char* msg = "what do ya want for nothing?";
        Bytes mb(msg, msg + strlen(msg));
        Crypto_DataBlob mi = blob(mb), o{};
        if (!rc) rc = OH_CryptoMac_Update(m, &mi);
        if (!rc) rc = OH_CryptoMac_Final(m, &o);
        r = rc ? "mac=" + std::to_string(rc) : hex(o.data, o.len);
        if (!rc) r += (r == "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843" ? " MATCH" : " MISMATCH");
        OH_Crypto_FreeDataBlob(&o);
        OH_CryptoMac_Destroy(m);
    }
    OH_CryptoSymKey_Destroy(k);
    OH_CryptoSymKeyGenerator_Destroy(g);
    return r;
}

// Runs one AES-256-GCM op. For decrypt, tagMode selects how the tag is passed.
// Returns rc, fills out with update+final output.
const char* gStep = "";
int gcmRun(Crypto_CipherMode mode, const Bytes& keyB, const Bytes& iv, const Bytes& aad, const Bytes& in,
           const Bytes* tag, Bytes& out) {
    OH_CryptoSymKeyGenerator* g = nullptr;
    OH_CryptoSymKey* k = nullptr;
    OH_CryptoSymCipher* c = nullptr;
    OH_CryptoSymCipherParams* p = nullptr;
    int rc = OH_CryptoSymKeyGenerator_Create("AES256", &g);
    Crypto_DataBlob kb = blob(keyB), ivb = blob(iv), aadb = blob(aad);
    if (!rc) { gStep = "OH_CryptoSymKeyGenerator_Convert"; rc = OH_CryptoSymKeyGenerator_Convert(g, &kb, &k); }
    if (!rc) { gStep = "OH_CryptoSymCipher_Create"; rc = OH_CryptoSymCipher_Create("AES256|GCM|NoPadding", &c); }
    if (!rc) { gStep = "OH_CryptoSymCipherParams_Create"; rc = OH_CryptoSymCipherParams_Create(&p); }
    if (!rc) { gStep = "OH_CryptoSymCipherParams_SetParam"; rc = OH_CryptoSymCipherParams_SetParam(p, CRYPTO_IV_DATABLOB, &ivb); }
    if (!rc && !aad.empty()) { gStep = "SetParam(AAD)"; rc = OH_CryptoSymCipherParams_SetParam(p, CRYPTO_AAD_DATABLOB, &aadb); }
    Crypto_DataBlob tb{};
    Bytes zeroTag(16, 0);
    if (!rc && mode == CRYPTO_ENCRYPT_MODE) tag = &zeroTag;  // probe: does encrypt Init want a (dummy) tag?
    if (!rc && tag) { gStep = "SetParam(TAG)"; tb = blob(*tag); rc = OH_CryptoSymCipherParams_SetParam(p, CRYPTO_TAG_DATABLOB, &tb); }
    if (!rc) { gStep = "OH_CryptoSymCipher_Init"; rc = OH_CryptoSymCipher_Init(c, mode, k, p); }
    Crypto_DataBlob inb = blob(in), u{}, f{};
    if (!rc) { gStep = "OH_CryptoSymCipher_Update"; rc = OH_CryptoSymCipher_Update(c, &inb, &u); }
    if (!rc) { gStep = "OH_CryptoSymCipher_Final"; rc = OH_CryptoSymCipher_Final(c, nullptr, &f); }
    if (!rc) {
        out.assign(u.data, u.data + u.len);
        out.insert(out.end(), f.data, f.data + f.len);
    }
    OH_Crypto_FreeDataBlob(&u);
    OH_Crypto_FreeDataBlob(&f);
    if (p) OH_CryptoSymCipherParams_Destroy(p);
    if (c) OH_CryptoSymCipher_Destroy(c);
    if (k) OH_CryptoSymKey_Destroy(k);
    if (g) OH_CryptoSymKeyGenerator_Destroy(g);
    return rc;
}

// NIST GCM test case 15 (AES-256, 96-bit IV, no AAD, 64-byte plaintext).
std::string probeGcm() {
    Bytes key = fromHex("feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308");
    Bytes iv = fromHex("cafebabefacedbaddecaf888");
    Bytes pt = fromHex("d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a72"
                       "1c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b391aafd255");
    const std::string wantCt = "522dc1f099567d07f47f37a32a84427d643a8cdcbfe5c0c97598a2bd2555d1aa"
                               "8cb08e48590dbb3da7b08b1056828838c5f61e6393ba7a0abcc9f662898015ad";
    const std::string wantTag = "b094dac5d93471bdec1a502270e3cc6c";
    Bytes aad, enc;
    int rc = gcmRun(CRYPTO_ENCRYPT_MODE, key, iv, aad, pt, nullptr, enc);
    if (rc) return "encrypt rc=" + std::to_string(rc) + " step=" + gStep;
    std::string r = "enc_out_len=" + std::to_string(enc.size()) + " (pt_len=64) ";
    std::string eh = hex(enc.data(), enc.size());
    r += eh == wantCt + wantTag ? "ct||tag MATCH; " : (eh.rfind(wantCt, 0) == 0 ? "ct MATCH tag-not-appended; " : "NIST MISMATCH; ");
    // Decrypt: separate tag param
    Bytes ct(enc.begin(), enc.begin() + 64), tag = fromHex(wantTag.c_str()), dec;
    rc = gcmRun(CRYPTO_DECRYPT_MODE, key, iv, aad, ct, &tag, dec);
    r += "decrypt(tag param) rc=" + std::to_string(rc) + (rc == 0 && dec == pt ? " roundtrip OK; " : " FAIL; ");
    // Decrypt: tag appended to ciphertext, no tag param
    Bytes combined = ct;
    combined.insert(combined.end(), tag.begin(), tag.end());
    dec.clear();
    rc = gcmRun(CRYPTO_DECRYPT_MODE, key, iv, aad, combined, nullptr, dec);
    r += "decrypt(ct||tag, no param) rc=" + std::to_string(rc) + (rc == 0 && dec == pt ? " roundtrip OK; " : " FAIL; ");
    // Tamper: flip tag bit must fail
    Bytes bad = tag; bad[0] ^= 1; dec.clear();
    rc = gcmRun(CRYPTO_DECRYPT_MODE, key, iv, aad, ct, &bad, dec);
    r += "tamper rc=" + std::to_string(rc) + (rc != 0 ? " rejected" : " ACCEPTED(bad)");
    // With AAD: roundtrip OK, wrong AAD rejected.
    Bytes aad2 = {'a', 'b', 'c'}, enc2, dec2;
    rc = gcmRun(CRYPTO_ENCRYPT_MODE, key, iv, aad2, pt, nullptr, enc2);
    Bytes ct2(enc2.begin(), enc2.begin() + 64), tag2(enc2.begin() + 64, enc2.end());
    int rcOk = rc ? rc : gcmRun(CRYPTO_DECRYPT_MODE, key, iv, aad2, ct2, &tag2, dec2);
    Bytes aad3 = {'a', 'b', 'd'};
    int rcBad = gcmRun(CRYPTO_DECRYPT_MODE, key, iv, aad3, ct2, &tag2, dec2);
    r += "; with-AAD enc_len=" + std::to_string(enc2.size()) + " dec rc=" + std::to_string(rcOk) +
         " wrong-AAD rc=" + std::to_string(rcBad);
    return r;
}

std::string probeRandom() {
    OH_CryptoRand* rnd = nullptr;
    int rc = OH_CryptoRand_Create(&rnd);
    if (rc) return "create=" + std::to_string(rc);
    Crypto_DataBlob a{}, b{};
    rc = OH_CryptoRand_GenerateRandom(rnd, 16, &a);
    if (!rc) rc = OH_CryptoRand_GenerateRandom(rnd, 16, &b);
    std::string r = rc ? "gen=" + std::to_string(rc)
                       : (a.len == 16 && b.len == 16 && memcmp(a.data, b.data, 16) ? "OK 16B distinct" : "BAD");
    OH_Crypto_FreeDataBlob(&a);
    OH_Crypto_FreeDataBlob(&b);
    OH_CryptoRand_Destroy(rnd);
    return r;
}

std::string probeAsset() {
    Bytes alias = {'p', 'r', 'o', 'b', 'e'}, secret = {1, 2, 3, 4, 5, 6, 7, 8};
    auto bl = [](Bytes& b) { Asset_Blob v{static_cast<uint32_t>(b.size()), b.data()}; return v; };
    Asset_Attr add[3] = {};
    add[0].tag = ASSET_TAG_ALIAS; add[0].value.blob = bl(alias);
    add[1].tag = ASSET_TAG_SECRET; add[1].value.blob = bl(secret);
    add[2].tag = ASSET_TAG_ACCESSIBILITY; add[2].value.u32 = ASSET_ACCESSIBILITY_DEVICE_FIRST_UNLOCKED;
    Asset_Attr q[2] = {};
    q[0].tag = ASSET_TAG_ALIAS; q[0].value.blob = bl(alias);
    q[1].tag = ASSET_TAG_RETURN_TYPE; q[1].value.u32 = ASSET_RETURN_ALL;
    Asset_Attr rm[1] = {q[0]};
    OH_Asset_Remove(rm, 1);  // clear leftovers
    std::string r;
    int rc = OH_Asset_Add(add, 3);
    r += fmt("add=%d ", rc);
    Asset_ResultSet rs{};
    rc = OH_Asset_Query(q, 2, &rs);
    r += fmt("query=%d ", rc);
    if (rc == 0 && rs.count == 1) {
        Asset_Attr* s = OH_Asset_ParseAttr(&rs.results[0], ASSET_TAG_SECRET);
        r += (s && s->value.blob.size == secret.size() && !memcmp(s->value.blob.data, secret.data(), secret.size()))
                  ? "secret roundtrip OK " : "secret MISMATCH ";
    } else {
        r += fmt("count=%d ", static_cast<int>(rs.count));
    }
    OH_Asset_FreeResultSet(&rs);
    rc = OH_Asset_Remove(rm, 1);
    r += fmt("remove=%d ", rc);
    rc = OH_Asset_Query(q, 2, &rs);
    r += fmt("query_after_remove=%d (expect 24000002)", rc);
    OH_Asset_FreeResultSet(&rs);
    return r;
}

OH_AudioData_Callback_Result silence(OH_AudioRenderer*, void*, void* data, int32_t size) {
    memset(data, 0, static_cast<size_t>(size));
    return AUDIO_DATA_CALLBACK_RESULT_VALID;
}

std::string probeAudio(int rate, OH_AudioStream_LatencyMode mode) {
    OH_AudioStreamBuilder* b = nullptr;
    int rc = OH_AudioStreamBuilder_Create(&b, AUDIOSTREAM_TYPE_RENDERER);
    if (rc) return "builder=" + std::to_string(rc);
    OH_AudioStreamBuilder_SetSamplingRate(b, rate);
    OH_AudioStreamBuilder_SetChannelCount(b, 2);
    OH_AudioStreamBuilder_SetSampleFormat(b, AUDIOSTREAM_SAMPLE_F32LE);
    OH_AudioStreamBuilder_SetEncodingType(b, AUDIOSTREAM_ENCODING_TYPE_RAW);
    OH_AudioStreamBuilder_SetRendererInfo(b, AUDIOSTREAM_USAGE_MUSIC);
    OH_AudioStreamBuilder_SetLatencyMode(b, mode);
    OH_AudioStreamBuilder_SetRendererWriteDataCallback(b, silence, nullptr);
    OH_AudioRenderer* r = nullptr;
    rc = OH_AudioStreamBuilder_GenerateRenderer(b, &r);
    std::string s = fmt("generate=%d", rc);
    if (rc == 0) {
        int32_t gr = 0, fs = 0;
        OH_AudioStream_SampleFormat gf{};
        OH_AudioStream_LatencyMode gm{};
        OH_AudioRenderer_GetSamplingRate(r, &gr);
        OH_AudioRenderer_GetSampleFormat(r, &gf);
        OH_AudioRenderer_GetLatencyMode(r, &gm);
        OH_AudioRenderer_GetFrameSizeInCallback(r, &fs);
        int src = OH_AudioRenderer_Start(r);
        usleep(200 * 1000);
        s += fmt(" actual_rate=%d", gr) + fmt(" actual_fmt=%d(4=F32LE)", static_cast<int>(gf)) +
             fmt(" actual_mode=%d(0=normal,1=fast)", static_cast<int>(gm)) + fmt(" frame=%d", fs) +
             fmt(" start=%d", src);
        OH_AudioRenderer_Stop(r);
        OH_AudioRenderer_Release(r);
    }
    OH_AudioStreamBuilder_Destroy(b);
    return s;
}

std::string probeConnect(int port) {
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) return "socket failed";
    timeval tv{3, 0};
    setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof tv);
    sockaddr_in a{};
    a.sin_family = AF_INET;
    a.sin_port = htons(static_cast<uint16_t>(port));
    inet_pton(AF_INET, "127.0.0.1", &a.sin_addr);
    std::string r;
    if (connect(fd, reinterpret_cast<sockaddr*>(&a), sizeof a) != 0) r = fmt("connect failed errno=%d", errno);
    else r = fmt("connected, sent=%d", static_cast<int>(send(fd, "hi", 2, 0)));
    close(fd);
    return r;
}

napi_value Probe(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value arg = nullptr;
    napi_get_cb_info(env, info, &argc, &arg, nullptr, nullptr);
    int32_t port = 0;
    if (argc > 0) napi_get_value_int32(env, arg, &port);
    std::string rep = "ok\n";
    if (port > 0) rep += "connect: " + probeConnect(port) + "\n";
    rep += "sha256: " + probeSha256() + "\n";
    rep += "hmac-sha256: " + probeHmac() + "\n";
    rep += "aes256gcm: " + probeGcm() + "\n";
    rep += "random: " + probeRandom() + "\n";
    rep += "asset: " + probeAsset() + "\n";
    for (int rate : {48000, 96000}) {
        rep += fmt("ohaudio f32 normal %d: ", rate) + probeAudio(rate, AUDIOSTREAM_LATENCY_MODE_NORMAL) + "\n";
        rep += fmt("ohaudio f32 fast %d: ", rate) + probeAudio(rate, AUDIOSTREAM_LATENCY_MODE_FAST) + "\n";
    }
    napi_value out;
    napi_create_string_utf8(env, rep.c_str(), rep.size(), &out);
    return out;
}

napi_value Init(napi_env env, napi_value exports) {
    napi_property_descriptor desc[] = {{"probe", nullptr, Probe, nullptr, nullptr, nullptr, napi_default, nullptr}};
    napi_define_properties(env, exports, 1, desc);
    return exports;
}

napi_module mod = {1, 0, nullptr, Init, "anpod_core", nullptr, {nullptr}};

}  // namespace

extern "C" __attribute__((constructor)) void RegisterAnpodCore() { napi_module_register(&mod); }
