using System;
using System.Globalization;
using System.IO;
using System.Text;

namespace AnPod.TestBench
{
    public static class MeasurementChecks
    {
        private const int Rate = 48000;
        private const ulong QpcPerSecond = 10000000UL;
        private const int TimestampErrorFlag = 0x4;
        private const int DiscontinuityFlag = 0x1;

        public static void Run()
        {
            CheckKnownDelayWithIndependentClocksOffsetsMissingPulseAndIntegrityFlags();
            CheckAmbiguousMicrophonePeaksAreReported();
            CheckNoiseAndSilenceDoNotProduceMatches();
            CheckUnsupportedFormatIsRejected();
            CheckDifferentSampleRatesAreRejectedWithReason();
            CheckTruncatedPacketIsRejected();
        }

        private static void CheckKnownDelayWithIndependentClocksOffsetsMissingPulseAndIntegrityFlags()
        {
            string directory = NewDirectory();
            try
            {
                double[] pulseTimes = new double[] { 0.8, 3.0, 5.2, 7.4 };
                int loopOffset = 7200;
                ulong loopBase = 100UL * QpcPerSecond;
                ulong micBase = loopBase + QpcPerSecond / 4;
                double[] loop = NewSamples(24.0 * Rate);
                double[] mic = NewSamples(24.0 * Rate);
                double[] chirp = MakeChirp(Rate);
                int timestampFrame = -1;
                for (int i = 0; i < pulseTimes.Length; i++)
                {
                    int loopStart = loopOffset + (int)Math.Round(pulseTimes[i] * Rate);
                    AddChirp(loop, loopStart, chirp);
                    if (i == 1)
                        continue;
                    double loopTimeAtStart = (double)loopStart / Rate;
                    double micStartTime = (double)loopBase / QpcPerSecond + loopTimeAtStart + 0.035 - (double)micBase / QpcPerSecond;
                    int micStart = (int)Math.Round(micStartTime * Rate);
                    AddChirp(mic, micStart, chirp);
                    if (i == 2)
                        timestampFrame = micStart;
                }

                WriteMetadata(directory, Rate, Rate, 32, 32, "formal");
                int discontinuityFrame = loopOffset + (int)Math.Round(pulseTimes[3] * Rate);
                WritePackets(Path.Combine(directory, "loopback.pkt"), loop, loopBase, discontinuityFrame, DiscontinuityFlag);
                WritePackets(Path.Combine(directory, "microphone.pkt"), mic, micBase, timestampFrame, TimestampErrorFlag);

                MeasurementResult result = Measurement.Analyze(directory);
                Assert(result.LoopbackSampleRate == Rate && result.MicrophoneSampleRate == Rate, "reported stream sample rates");
                Assert(result.ExpectedPulseCount == 10 && result.DetectedSourceCount == 4, "formal expected and detected source counts are separate");
                Assert(result.Warnings.Exists(delegate(string warning) { return warning.IndexOf("differs from expected 10", StringComparison.Ordinal) >= 0; }), "missing formal source pulses are explicitly warned");
                Assert(result.PulseCount == 4, "four source chirps detected");
                Assert(result.MatchedCount == 3, "one microphone pulse is missing and three are assigned");
                Assert(result.ValidCount == 1, "timestamp-error and discontinuity segments are invalid");
                Assert(result.Samples[1].DelayMs == null && result.Samples[1].Reason.IndexOf("No unused microphone", StringComparison.Ordinal) >= 0, "missing pulse reported");
                Assert(result.Samples[2].TimestampError && result.Samples[2].Reason.IndexOf("timestamp-error", StringComparison.OrdinalIgnoreCase) >= 0, "timestamp error reported on affected pulse");
                Assert(result.Samples[3].Discontinuity && result.Samples[3].Reason.IndexOf("discontinuity", StringComparison.OrdinalIgnoreCase) >= 0, "discontinuity reported on affected pulse");
                Assert(result.Samples[0].Valid && Math.Abs(result.Samples[0].DelayMs.Value - 35.0) < 0.2, "known QPC delay recovered despite independent startup QPC and sample offsets");
                Assert(result.MedianMs.HasValue && Math.Abs(result.MedianMs.Value - 35.0) < 0.2, "summary includes only valid reference measurements");
                Assert(result.Interpretation.IndexOf("must not be subtracted", StringComparison.OrdinalIgnoreCase) >= 0, "independent PC reference is explicitly separate from phone latency");
            }
            finally
            {
                Directory.Delete(directory, true);
            }
        }

        private static void CheckNoiseAndSilenceDoNotProduceMatches()
        {
            string directory = NewDirectory();
            try
            {
                int loopOffset = 2400;
                ulong loopBase = 200UL * QpcPerSecond;
                ulong micBase = loopBase + QpcPerSecond / 20;
                double[] loop = NewSamples(5.5 * Rate);
                double[] mic = NewSamples(5.5 * Rate);
                double[] chirp = MakeChirp(Rate);
                AddChirp(loop, loopOffset + (int)(0.7 * Rate), chirp);
                AddChirp(loop, loopOffset + (int)(2.9 * Rate), chirp);
                Random random = new Random(1907);
                for (int i = 0; i < mic.Length; i++)
                    mic[i] = (random.NextDouble() - 0.5) * 0.03;
                WriteMetadata(directory, Rate, Rate, 32, 32, "precheck");
                WritePackets(Path.Combine(directory, "loopback.pkt"), loop, loopBase, -1, 0);
                WritePackets(Path.Combine(directory, "microphone.pkt"), mic, micBase, -1, 0);
                MeasurementResult result = Measurement.Analyze(directory);
                Assert(result.ExpectedPulseCount == 2 && result.DetectedSourceCount == 2, "precheck expected and detected source counts");
                Assert(result.PulseCount == 2, "loopback chirps detected in noise-only microphone capture");
                Assert(result.MatchedCount == 0 && result.ValidCount == 0, "noise-only microphone capture has no match");
                Assert(result.Samples[0].DelayMs == null && result.Samples[0].Reason.Length > 0, "no-match reason is present");
                Assert(result.MedianMs == null && result.Warnings.Count > 0, "no-match result cannot be reported as PASS");
            }
            finally
            {
                Directory.Delete(directory, true);
            }
        }

        private static void CheckAmbiguousMicrophonePeaksAreReported()
        {
            string directory = NewDirectory();
            try
            {
                ulong loopBase = 300UL * QpcPerSecond;
                ulong micBase = loopBase + QpcPerSecond / 10;
                double[] loop = NewSamples(4.0 * Rate);
                double[] mic = NewSamples(4.0 * Rate);
                double[] chirp = MakeChirp(Rate);
                int firstSource = (int)(0.6 * Rate);
                int secondSource = (int)(2.5 * Rate);
                AddChirp(loop, firstSource, chirp);
                AddChirp(loop, secondSource, chirp);
                int firstMic = (int)Math.Round((0.6 + 0.04 - 0.1) * Rate);
                int firstEcho = (int)Math.Round((0.6 + 0.50 - 0.1) * Rate);
                int secondMic = (int)Math.Round((2.5 + 0.04 - 0.1) * Rate);
                AddChirp(mic, firstMic, chirp);
                AddChirp(mic, firstEcho, chirp);
                AddChirp(mic, secondMic, chirp);
                WriteMetadata(directory, Rate, Rate, 32, 32, "precheck");
                WritePackets(Path.Combine(directory, "loopback.pkt"), loop, loopBase, -1, 0);
                WritePackets(Path.Combine(directory, "microphone.pkt"), mic, micBase, -1, 0);
                MeasurementResult result = Measurement.Analyze(directory);
                Assert(result.ExpectedPulseCount == 2 && result.DetectedSourceCount == 2, "ambiguous precheck keeps expected and detected counts");
                Assert(result.MatchedCount == 2 && result.ValidCount == 1, "ambiguous pulse is not silently accepted");
                Assert(result.Samples[0].AmbiguousCandidateCount == 2 && result.Samples[0].Reason.IndexOf("Ambiguous", StringComparison.Ordinal) >= 0, "multiple microphone candidates are explicit");
            }
            finally
            {
                Directory.Delete(directory, true);
            }
        }

        private static void CheckUnsupportedFormatIsRejected()
        {
            string directory = NewDirectory();
            try
            {
                WriteMetadata(directory, Rate, Rate, 16, 16, "formal");
                AssertThrows<InvalidDataException>(delegate { Measurement.Analyze(directory); }, "unsupported PCM format");
            }
            finally
            {
                Directory.Delete(directory, true);
            }
        }

        private static void CheckTruncatedPacketIsRejected()
        {
            string directory = NewDirectory();
            try
            {
                WriteMetadata(directory, Rate, Rate, 32, 32, "formal");
                File.WriteAllBytes(Path.Combine(directory, "loopback.pkt"), new byte[] { 1, 2, 3, 4, 5, 6, 7 });
                File.WriteAllBytes(Path.Combine(directory, "microphone.pkt"), new byte[0]);
                AssertThrows<InvalidDataException>(delegate { Measurement.Analyze(directory); }, "truncated packet header");
            }
            finally
            {
                Directory.Delete(directory, true);
            }
        }

        private static void CheckDifferentSampleRatesAreRejectedWithReason()
        {
            string directory = NewDirectory();
            try
            {
                WriteMetadata(directory, Rate, 44100, 32, 32, "precheck");
                try
                {
                    Measurement.Analyze(directory);
                }
                catch (InvalidDataException error)
                {
                    Assert(error.Message.IndexOf("sample rates differ", StringComparison.OrdinalIgnoreCase) >= 0 && error.Message.IndexOf("waveform refinement", StringComparison.OrdinalIgnoreCase) >= 0, "different-rate rejection explains unsupported analysis");
                    return;
                }
                throw new InvalidOperationException("Different sample rates were not rejected.");
            }
            finally
            {
                Directory.Delete(directory, true);
            }
        }

        private static string NewDirectory()
        {
            string path = Path.Combine(Path.GetTempPath(), "AnPod.TestBench." + Path.GetRandomFileName());
            Directory.CreateDirectory(path);
            return path;
        }

        private static double[] NewSamples(double frames)
        {
            return new double[checked((int)frames)];
        }

        private static double[] MakeChirp(int rate)
        {
            int count = rate / 10;
            double[] result = new double[count];
            double seconds = (double)count / rate;
            for (int i = 0; i < count; i++)
            {
                double time = (double)i / rate;
                double phase = 2.0 * Math.PI * (700.0 * time + (8500.0 - 700.0) * time * time / (2.0 * seconds));
                double window = 0.5 - 0.5 * Math.Cos(2.0 * Math.PI * time / seconds);
                result[i] = 0.4 * window * Math.Sin(phase);
            }
            return result;
        }

        private static void AddChirp(double[] samples, int start, double[] chirp)
        {
            if (start < 0 || start + chirp.Length > samples.Length)
                throw new InvalidOperationException("Synthetic chirp does not fit in its stream.");
            for (int i = 0; i < chirp.Length; i++)
                samples[start + i] += chirp[i];
        }

        private static void WriteMetadata(string directory, int loopRate, int micRate, int loopBits, int micBits, string mode)
        {
            StringBuilder text = new StringBuilder();
            text.AppendLine("format=AnPod acoustic latency probe v1");
            text.AppendLine("mode=" + mode);
            text.AppendLine("qpcUnits=100ns");
            text.AppendLine("packetHeader=uint64 qpc100ns, uint32 frames, uint32 flags; followed by frames*blockAlign PCM bytes");
            AppendFormat(text, "loopback", loopRate, loopBits);
            AppendFormat(text, "microphone", micRate, micBits);
            File.WriteAllText(Path.Combine(directory, "metadata.txt"), text.ToString(), Encoding.UTF8);
        }

        private static void AppendFormat(StringBuilder text, string name, int rate, int bits)
        {
            text.AppendLine(name + ".endpointName=Synthetic");
            text.AppendLine(name + ".endpointId=synthetic");
            text.AppendLine(name + ".formatTag=65534");
            text.AppendLine(name + ".sampleRate=" + rate.ToString(CultureInfo.InvariantCulture));
            text.AppendLine(name + ".channels=2");
            text.AppendLine(name + ".bitsPerSample=" + bits.ToString(CultureInfo.InvariantCulture));
            text.AppendLine(name + ".blockAlign=8");
            text.AppendLine(name + ".extraBytes=22");
            text.AppendLine(name + ".validBits=" + bits.ToString(CultureInfo.InvariantCulture));
            text.AppendLine(name + ".subFormat=" + (bits == 32 ? "00000003-0000-0010-8000-00aa00389b71" : "00000001-0000-0010-8000-00aa00389b71"));
            text.AppendLine(name + ".channelMask=0x00000003");
        }

        private static void WritePackets(string path, double[] samples, ulong baseQpc, int flagStartFrame, int flag)
        {
            const int packetFrames = 480;
            using (BinaryWriter writer = new BinaryWriter(File.Open(path, FileMode.Create, FileAccess.Write, FileShare.None)))
            {
                for (int start = 0; start < samples.Length; start += packetFrames)
                {
                    int frames = Math.Min(packetFrames, samples.Length - start);
                    ulong qpc = baseQpc + (ulong)Math.Round((double)start * QpcPerSecond / Rate, MidpointRounding.ToEven);
                    uint flags = flag != 0 && flagStartFrame >= start && flagStartFrame < start + frames ? (uint)flag : 0U;
                    writer.Write(qpc);
                    writer.Write((uint)frames);
                    writer.Write(flags);
                    for (int i = 0; i < frames; i++)
                    {
                        float value = (float)samples[start + i];
                        writer.Write(value);
                        writer.Write(value);
                    }
                }
            }
        }

        private static void AssertThrows<T>(Action action, string name) where T : Exception
        {
            try
            {
                action();
            }
            catch (T)
            {
                return;
            }
            throw new InvalidOperationException("Expected rejection for " + name + ".");
        }

        private static void Assert(bool condition, string name)
        {
            if (!condition)
                throw new InvalidOperationException("Measurement check failed: " + name + ".");
        }
    }
}
