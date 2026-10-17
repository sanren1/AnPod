package app.anpod.receiver;

import java.nio.charset.StandardCharsets;
import java.util.Arrays;

import javax.crypto.Mac;
import javax.crypto.spec.SecretKeySpec;

/**
 * Discovery MACs shared with the desktop client. Distinct labels keep these values
 * separate from every other use of the binding secret.
 */
final class DiscoveryAuth {
    private static final byte[] TAG_LABEL = "anpod-discovery-tag-v3".getBytes(StandardCharsets.US_ASCII);
    private static final byte[] PROBE_LABEL = "anpod-probe-v3".getBytes(StandardCharsets.US_ASCII);
    private static final byte[] CANDIDATE_LABEL = "anpod-candidate-v3".getBytes(StandardCharsets.US_ASCII);

    private DiscoveryAuth() { }

    static byte[] tag(byte[] secret) {
        return Arrays.copyOf(hmac(secret, TAG_LABEL), 8);
    }

    static byte[] probeMac(byte[] secret, byte[] nonce, byte[] tag) {
        return Arrays.copyOf(hmac(secret, PROBE_LABEL, nonce, tag), 16);
    }

    static byte[] candidateMac(byte[] secret, byte[] nonce, byte[] tag, int port) {
        return Arrays.copyOf(hmac(secret, CANDIDATE_LABEL, nonce, tag,
                new byte[] { (byte) (port >>> 8), (byte) port }), 16);
    }

    private static byte[] hmac(byte[] secret, byte[]... parts) {
        try {
            Mac mac = Mac.getInstance("HmacSHA256");
            mac.init(new SecretKeySpec(secret, "HmacSHA256"));
            for (byte[] part : parts) mac.update(part);
            return mac.doFinal();
        } catch (java.security.GeneralSecurityException error) {
            throw new IllegalStateException("HmacSHA256 unavailable", error);
        }
    }
}
