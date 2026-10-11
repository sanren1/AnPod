import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.HexFormat;
import javax.crypto.Cipher;
import javax.crypto.spec.GCMParameterSpec;
import javax.crypto.spec.SecretKeySpec;

public final class AnPodUdpVectorCheck {
    public static void main(String[] args) throws Exception {
        byte[] key = new byte[32];
        for (int i = 0; i < key.length; i++) key[i] = (byte) i;
        long sessionId = 0x0102030405060708L;
        long sequence = 0x1112131415161718L;
        long frameIndex = 0x2122232425262728L;
        byte[] payload = {0x00, 0x01, 0x7f, (byte) 0x80, (byte) 0xfe, (byte) 0xff, 0x42};
        byte[] header = ByteBuffer.allocate(34).order(ByteOrder.BIG_ENDIAN)
                .put(new byte[] {'A', 'N', 'P', 'D'})
                .put((byte) 1).put((byte) 1).putShort((short) 0)
                .putLong(sessionId).putLong(sequence).putLong(frameIndex)
                .putShort((short) payload.length).array();
        byte[] nonce = ByteBuffer.allocate(12).order(ByteOrder.BIG_ENDIAN)
                .putInt(0x414e5048).putLong(sequence).array();
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
        cipher.init(Cipher.ENCRYPT_MODE, new SecretKeySpec(key, "AES"), new GCMParameterSpec(128, nonce));
        cipher.updateAAD(header);
        byte[] encrypted = cipher.doFinal(payload);
        byte[] packet = ByteBuffer.allocate(header.length + encrypted.length).put(header).put(encrypted).array();
        String actual = HexFormat.of().formatHex(packet);
        String expected = "414e50440101000001020304050607081112131415161718212223242526272800073bd32562c2e2cdfe093c22223a9705d1ae457629baad04";
        if (!expected.equals(actual)) throw new AssertionError("AES-GCM packet mismatch: " + actual);
        System.out.println("Java JCA AES-256-GCM vector passed.");
    }
}
