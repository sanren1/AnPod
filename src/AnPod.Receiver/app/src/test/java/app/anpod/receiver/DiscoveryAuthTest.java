package app.anpod.receiver;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;

import java.util.Arrays;

import org.junit.Test;

/**
 * Pins the discovery MAC layout shared with the desktop client. A change here means the
 * Android and Rust implementations would stop authenticating each other.
 */
public final class DiscoveryAuthTest {
    private static final byte[] SECRET = range(32);
    private static final byte[] NONCE = range(16);
    private static final byte[] TAG = DiscoveryAuth.tag(SECRET);

    private static byte[] range(int size) {
        byte[] value = new byte[size];
        for (int i = 0; i < size; i++) {
            value[i] = (byte) i;
        }
        return value;
    }

    private static String hex(byte[] value) {
        StringBuilder text = new StringBuilder(value.length * 2);
        for (byte item : value) {
            text.append(String.format("%02x", item & 0xff));
        }
        return text.toString();
    }

    @Test
    public void tagIsTheStableEightByteHmac() {
        assertEquals(8, TAG.length);
        assertEquals("26414e36e4dc63e6", hex(TAG));
    }

    @Test
    public void probeMacBindsTheNonceAndTag() {
        assertEquals("83ec229f58b0088dd60051cb736e6db8", hex(DiscoveryAuth.probeMac(SECRET, NONCE, TAG)));

        byte[] otherNonce = NONCE.clone();
        otherNonce[0] ^= 1;
        assertFalse(Arrays.equals(DiscoveryAuth.probeMac(SECRET, NONCE, TAG),
                DiscoveryAuth.probeMac(SECRET, otherNonce, TAG)));
    }

    @Test
    public void candidateMacBindsTheAdvertisedPort() {
        assertEquals("cc22b8c876869dc9aabbd5b84d90c466",
                hex(DiscoveryAuth.candidateMac(SECRET, NONCE, TAG, 0x1234)));
        assertFalse(Arrays.equals(DiscoveryAuth.candidateMac(SECRET, NONCE, TAG, 0x1234),
                DiscoveryAuth.candidateMac(SECRET, NONCE, TAG, 0x1235)));
    }
}
