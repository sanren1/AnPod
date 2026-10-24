package app.anpod.receiver;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import com.anpod.server.Float32LeCodec;
import com.anpod.server.FloatPcmFrameQueue;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import javax.crypto.Cipher;
import javax.crypto.spec.GCMParameterSpec;
import javax.crypto.spec.SecretKeySpec;
import org.junit.Test;

public final class Float32LeCodecTest {
    @Test public void v2FloatProfilesRoundTripTheirExactDescriptors() {
        for (int profile = 4; profile <= 5; profile++) {
            UdpPacketDecoder.AudioFormatV2 format = UdpPacketDecoder.AudioFormatV2.forProfile(profile, 1);
            assertEquals(profile == 4 ? 48_000 : 96_000, format.sampleRate);
            assertEquals(3, format.sampleFormat);
            assertEquals(32, format.validBits);
            assertEquals(4, format.bytesPerSample);
            assertEquals(8, format.frameBytes);
            UdpPacketDecoder.AudioFormatV2 decoded = UdpPacketDecoder.AudioFormatV2.decode(format.encode());
            assertEquals(profile, decoded.profile);
            assertEquals(1, decoded.mode);
        }
    }

    @Test public void decodePreservesFiniteBitsAndRejectsNonFiniteSamples() {
        byte[] samples = {0, 0, 0, (byte) 0x80, 0, 0, 0, 0x3f};
        float[] decoded = new float[2];
        Float32LeCodec.decode(samples, 0, 2, decoded, 0);
        assertEquals(0x80000000, Float.floatToRawIntBits(decoded[0]));
        assertEquals(0x3f000000, Float.floatToRawIntBits(decoded[1]));
        assertTrue(Float.isFinite(decoded[0]) && Float.isFinite(decoded[1]));

        assertThrows(IllegalArgumentException.class,
                () -> Float32LeCodec.decode(new byte[] {0, 0, (byte) 0x80, 0x7f}, 0, 1,
                        new float[1], 0));
        assertThrows(IllegalArgumentException.class,
                () -> Float32LeCodec.decode(new byte[] {0, 0, (byte) 0x80, (byte) 0xff}, 0, 1,
                        new float[1], 0));
    }

    @Test public void authenticatedIngressKeepsFloatBitsAndRejectsNonFiniteAudio() throws Exception {
        byte[] key = new byte[32];
        UdpPacketDecoder.AudioFormatV2 format = UdpPacketDecoder.AudioFormatV2.forProfile(4, 0);
        FloatPcmFrameQueue queue = new FloatPcmFrameQueue(48_000, true, 143);
        try (AnpdAudioIngress ingress = AnpdAudioIngress.aoa(7, key, format, null, queue)) {
            byte[] descriptor = packet(key, 0, 0, UdpPacketDecoder.FORMAT, format.encode());
            assertEquals(AnpdAudioIngress.Result.FORMAT, ingress.accept(descriptor, descriptor.length));

            byte[] valid = packet(key, 1, 0, UdpPacketDecoder.AUDIO,
                    new byte[] {0, 0, 0, (byte) 0x80, 0, 0, 0, 0x3f});
            assertEquals(AnpdAudioIngress.Result.AUDIO, ingress.accept(valid, valid.length));
            float[] decoded = new float[2];
            assertEquals(1, queue.poll(decoded, 0, 1));
            assertEquals(0x80000000, Float.floatToRawIntBits(decoded[0]));
            assertEquals(0x3f000000, Float.floatToRawIntBits(decoded[1]));

            byte[] nonFinite = packet(key, 2, 1, UdpPacketDecoder.AUDIO,
                    new byte[] {0, 0, (byte) 0x80, 0x7f, 0, 0, 0, 0});
            assertEquals(AnpdAudioIngress.Result.FORMAT_REJECTED,
                    ingress.accept(nonFinite, nonFinite.length));
            assertEquals(0, queue.queuedFrames());
        }
    }

    private static byte[] packet(byte[] key, long sequence, long frameIndex, int type, byte[] payload)
            throws Exception {
        byte[] header = ByteBuffer.allocate(UdpPacketDecoder.HEADER_BYTES).order(ByteOrder.BIG_ENDIAN)
                .put(new byte[] {'A', 'N', 'P', 'D'}).put((byte) 2).put((byte) type).putShort((short) 0)
                .putLong(7).putLong(sequence).putLong(frameIndex).putShort((short) payload.length).array();
        byte[] nonce = ByteBuffer.allocate(12).order(ByteOrder.BIG_ENDIAN)
                .putInt(0x414e5048).putLong(sequence).array();
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
        cipher.init(Cipher.ENCRYPT_MODE, new SecretKeySpec(key, "AES"), new GCMParameterSpec(128, nonce));
        cipher.updateAAD(header);
        byte[] sealed = cipher.doFinal(payload);
        byte[] datagram = new byte[header.length + sealed.length];
        System.arraycopy(header, 0, datagram, 0, header.length);
        System.arraycopy(sealed, 0, datagram, header.length, sealed.length);
        return datagram;
    }
}
