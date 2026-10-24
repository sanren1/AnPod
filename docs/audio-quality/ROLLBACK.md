# Audio feature rollback

This guide keeps the established LAN/UDP and USB/ADB paths available while
audio and AOA work remains experimental. It describes a user-visible recovery
path; it does not promise that an older APK can be installed over a newer one
without clearing app data. AOA product State/permission integration has offline
acceptance; no physical AOA recovery test has been performed.

## Recover playback without changing the source tree

1. Stop the current playback session from AnPod or Receiver.
2. For the existing routes, choose Standard (48 kHz / 16-bit) and the default
   AudioTrack output, then explicitly reconnect over USB/ADB or LAN/UDP.
   Unsupported audio formats fail visibly; AnPod does not silently change a
   format or transport within an authenticated session.
3. AOA is an explicit accessory session. The product flow does not switch the
   phone's USB mode, fall back to ADB, or reuse LAN background trust. To return
   to the original USB mode, disconnect/reconnect the cable and follow the
   phone's USB prompt. Exact recovery behavior depends on the phone and remains
   `NOT_TESTED` on the current product path.
4. If an experimental Oboe session fails, the current audio generation is
   closed. A later Oboe-requested session may report a same-format AudioTrack
   fallback; the user's engine preference is not silently rewritten. Selecting
   AudioTrack explicitly and starting a new session remains the direct local
   rollback.

## Roll back source changes safely

The development line was created from local source baseline
`a406cedacb4939e9647d6046fca4f09cfefb84f9`; `c299c805f8917df0f961b5174dff47fb074517e6`
is the historical investigation/preview snapshot. To inspect or rebuild that
baseline, use a separate worktree or checkout. Do not run `git reset --hard`
on a workspace that may contain user changes. Prefer a new revert commit for
an unwanted feature commit so later changes and local work remain visible.

Audio features are additive: Standard v1 on LAN/UDP and USB/ADB, the AudioTrack
default, and existing ADB driver configuration remain independent. No source
rollback should replace a driver, erase binding storage, or change the
published signing key.

## Receiver APK and signing limits

Receiver APK upgrades use Android package version codes and signing identity.
An older APK normally has a lower `versionCode`, so Android does not promise
an in-place downgrade even when it has the same release certificate. Do not
promise that such a downgrade preserves ANPB binding or preferences. If a
rollback requires uninstalling the newer app, app-private data and bindings
are removed and pairing must be repeated. A debug-signed APK and the
release-signed APK also cannot replace one another in place.

For a source-based rollback build, keep the existing release key and alias,
increment `versionCode` as required by the release process, and register the
new APK's exact hash, version and signer in `AUDITED_RECEIVER_ARTIFACTS`.
Follow [`packaging/SIGNING.md`](../../packaging/SIGNING.md); never rotate,
replace, print, or add the private key to the repository as a rollback step.

The signed Receiver 0.17.0/code 34 candidate APK has been built and its
certificate verified. The portable candidate's `build-metadata.json` records
the source commit, `sourceInputsSha256`, APK SHA-256, signer, component hashes,
component source commits when their exact hashes are registered, and
whole-worktree dirty state. Portable
ZIP checks and checksum are retained in local release records. Receiver source-input
checks include tracked files and real untracked source, while excluding only
untracked generated files under `src/AnPod.Receiver/app/.cxx/`; tracked files
there remain inputs. The workspace may therefore be globally dirty from a
preserved local build cache while Receiver source inputs are unchanged. Such
metadata must report both facts accurately; a candidate from unreviewed source
changes is not a release.
