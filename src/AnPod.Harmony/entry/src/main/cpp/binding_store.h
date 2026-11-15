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
    static std::optional<BindingMaterial> find(const Id16& hostId);
    // Adds or overwrites the binding stored for material.hostId().
    static bool save(const BindingMaterial& material);
    // True when the binding is gone afterwards (removed now or never stored).
    static bool remove(const Id16& hostId);
    static std::vector<Id16> list();
};

}  // namespace anpod
