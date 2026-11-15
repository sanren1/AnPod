#include "binding_store.h"

#include "crypto.h"

#include <asset/asset_api.h>

#include <cstring>
#include <string>

namespace anpod {
namespace {

constexpr char kPrefix[] = "anpod.binding.";
constexpr size_t kPrefixLength = sizeof kPrefix - 1;
constexpr char kHex[] = "0123456789abcdef";

std::string alias(const Id16& hostId) {
    std::string out = kPrefix;
    for (uint8_t b : hostId) {
        out.push_back(kHex[b >> 4]);
        out.push_back(kHex[b & 0xf]);
    }
    return out;
}

std::optional<Id16> hostIdFromAlias(const Asset_Blob& blob) {
    if (blob.size != kPrefixLength + 32 || std::memcmp(blob.data, kPrefix, kPrefixLength) != 0) return std::nullopt;
    Id16 id;
    for (size_t i = 0; i < 16; i++) {
        uint8_t byte = 0;
        for (size_t j = 0; j < 2; j++) {
            const char* at = std::strchr(kHex, blob.data[kPrefixLength + i * 2 + j]);
            if (blob.data[kPrefixLength + i * 2 + j] == 0 || !at) return std::nullopt;
            byte = static_cast<uint8_t>(byte << 4 | (at - kHex));
        }
        id[i] = byte;
    }
    return id;
}

Asset_Attr bytesAttr(uint32_t tag, const void* data, size_t size) {
    Asset_Attr attr{};
    attr.tag = tag;
    attr.value.blob.size = static_cast<uint32_t>(size);
    attr.value.blob.data = static_cast<uint8_t*>(const_cast<void*>(data));
    return attr;
}

Asset_Attr numberAttr(uint32_t tag, uint32_t value) {
    Asset_Attr attr{};
    attr.tag = tag;
    attr.value.u32 = value;
    return attr;
}

}  // namespace

std::optional<BindingMaterial> BindingStore::find(const Id16& hostId) {
    std::string name = alias(hostId);
    Asset_Attr query[] = {bytesAttr(ASSET_TAG_ALIAS, name.data(), name.size()),
                          numberAttr(ASSET_TAG_RETURN_TYPE, ASSET_RETURN_ALL)};
    Asset_ResultSet results{};
    if (OH_Asset_Query(query, 2, &results) != ASSET_SUCCESS) return std::nullopt;
    std::optional<BindingMaterial> found;
    for (uint32_t i = 0; i < results.count; i++) {
        Asset_Attr* secret = OH_Asset_ParseAttr(&results.results[i], ASSET_TAG_SECRET);
        if (!secret) continue;
        Asset_Blob& blob = secret->value.blob;
        if (!found && blob.size == BindingMaterial::kBytes) {
            auto material = BindingMaterial::decode(std::span<const uint8_t, BindingMaterial::kBytes>(blob.data, blob.size));
            if (material && material->hostId() == hostId) found = std::move(material);
        }
        secureZero(blob.data, blob.size);
    }
    OH_Asset_FreeResultSet(&results);
    return found;
}

bool BindingStore::save(const BindingMaterial& material) {
    std::string name = alias(material.hostId());
    auto record = material.encode();
    Asset_Attr attrs[] = {bytesAttr(ASSET_TAG_ALIAS, name.data(), name.size()),
                          bytesAttr(ASSET_TAG_SECRET, record.data(), record.size()),
                          numberAttr(ASSET_TAG_ACCESSIBILITY, ASSET_ACCESSIBILITY_DEVICE_FIRST_UNLOCKED),
                          numberAttr(ASSET_TAG_CONFLICT_RESOLUTION, ASSET_CONFLICT_OVERWRITE)};
    bool ok = OH_Asset_Add(attrs, 4) == ASSET_SUCCESS;
    secureZero(record.data(), record.size());
    return ok;
}

bool BindingStore::remove(const Id16& hostId) {
    std::string name = alias(hostId);
    Asset_Attr query[] = {bytesAttr(ASSET_TAG_ALIAS, name.data(), name.size())};
    int32_t rc = OH_Asset_Remove(query, 1);
    return rc == ASSET_SUCCESS || rc == ASSET_NOT_FOUND;
}

std::vector<Id16> BindingStore::list() {
    Asset_Attr query[] = {numberAttr(ASSET_TAG_RETURN_TYPE, ASSET_RETURN_ATTRIBUTES)};
    Asset_ResultSet results{};
    std::vector<Id16> ids;
    if (OH_Asset_Query(query, 1, &results) != ASSET_SUCCESS) return ids;
    for (uint32_t i = 0; i < results.count; i++) {
        Asset_Attr* name = OH_Asset_ParseAttr(&results.results[i], ASSET_TAG_ALIAS);
        if (!name) continue;
        if (auto id = hostIdFromAlias(name->value.blob)) ids.push_back(*id);
    }
    OH_Asset_FreeResultSet(&results);
    return ids;
}

}  // namespace anpod
