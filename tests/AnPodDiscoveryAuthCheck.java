package app.anpod.receiver;

import java.util.Arrays;

/** Cross-language vector shared with src-tauri/src/discovery.rs. */
public final class AnPodDiscoveryAuthCheck {
    private AnPodDiscoveryAuthCheck() { }

    public static void main(String[] args) {
        byte[] secret = new byte[32];
        for (int index = 0; index < secret.length; index++) secret[index] = (byte) index;
        byte[] nonce = new byte[16];
        for (int index = 0; index < nonce.length; index++) nonce[index] = (byte) index;
        byte[] tag = DiscoveryAuth.tag(secret);
        expect(tag, "26414e36e4dc63e6", "tag");
        expect(DiscoveryAuth.probeMac(secret, nonce, tag), "83ec229f58b0088dd60051cb736e6db8", "probe mac");
        expect(DiscoveryAuth.candidateMac(secret, nonce, tag, 41_234), "a82f6568b5e977d37c46b22155edcba4", "candidate mac");
        byte[] otherSecret = new byte[32];
        Arrays.fill(otherSecret, (byte) 9);
        if (Arrays.equals(DiscoveryAuth.tag(otherSecret), tag)) throw new AssertionError("tags must differ per binding");
        System.out.println("DISCOVERY_AUTH_CHECK=PASS");
    }

    private static void expect(byte[] actual, String expectedHex, String label) {
        String actualHex = hex(actual);
        if (!actualHex.equals(expectedHex)) {
            throw new AssertionError(label + ": expected " + expectedHex + " but was " + actualHex);
        }
    }

    private static String hex(byte[] value) {
        StringBuilder text = new StringBuilder(value.length * 2);
        for (byte item : value) text.append(String.format("%02x", item));
        return text.toString();
    }
}
