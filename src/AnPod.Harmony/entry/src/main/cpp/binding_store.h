// Tablet-side binding storage in the Asset Store (asset_ndk): one asset per paired PC, alias
// "anpod.binding.<hostIdHex>", secret = the 69-byte ANPB record, accessible after first unlock.
// Never logs material; every plaintext buffer is zeroed after use.
#pragma once

#include "pairing_protocol.h"

#include <optional>
#include <vector>

namespace anpod {

class BindingStore {
public:
    // false on an Asset Store error (locked, service unavailable...): the caller must not treat that as
    // "not paired". true with an empty out means no binding is stored for hostId.
    static bool find(const Id16& hostId, std::optional<BindingMaterial>& out);
    // Adds or overwrites the binding stored for material.hostId().
    static bool save(const BindingMaterial& material);
    // True when the binding is gone afterwards (removed now or never stored).
    static bool remove(const Id16& hostId);
    // nullopt on an Asset Store error.
    static std::optional<std::vector<Id16>> list();
};

}  // namespace anpod
