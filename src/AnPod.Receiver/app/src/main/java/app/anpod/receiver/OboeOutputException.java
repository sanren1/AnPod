package app.anpod.receiver;

import java.io.IOException;

/** Failure reported by the experimental Oboe output bridge. */
final class OboeOutputException extends IOException {
    OboeOutputException(String message) { super(message); }
    OboeOutputException(String message, Throwable cause) { super(message, cause); }
}
