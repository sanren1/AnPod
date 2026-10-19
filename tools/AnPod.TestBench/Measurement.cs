using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Numerics;

namespace AnPod.TestBench
{
    [Serializable]
    public sealed class MeasurementResult
    {
        public int LoopbackSampleRate { get; set; }
        public int MicrophoneSampleRate { get; set; }
        public int? ExpectedPulseCount { get; set; }
        public int PulseCount { get; set; }
        public int DetectedSourceCount { get; set; }
        public int MatchedCount { get; set; }
        public int ValidCount { get; set; }
        public int MicrophonePeakCount { get; set; }
        public string MeasurementType { get; set; }
        public string Interpretation { get; set; }
        public double? MedianMs { get; set; }
        public double? P95Ms { get; set; }
        public double? MinMs { get; set; }
        public double? MaxMs { get; set; }
        public List<string> Warnings { get; set; }
        public List<MeasurementSample> Samples { get; set; }

        public MeasurementResult()
        {
            Warnings = new List<string>();
            Samples = new List<MeasurementSample>();
            MeasurementType = "Windows loopback-to-microphone acoustic interval";
            Interpretation = "Includes playback presentation, speaker/room acoustics, and microphone capture; it is not pure transport latency. The GUI should label the selected phone route or PC-only reference scene. An independent PC reference must not be subtracted from a phone measurement.";
        }
    }

    [Serializable]
    public sealed class MeasurementSample
    {
        public int PulseIndex { get; set; }
        public int LoopbackFrameIndex { get; set; }
        public int? MicrophoneFrameIndex { get; set; }
        public ulong LoopbackQpc100ns { get; set; }
        public ulong? MicrophoneQpc100ns { get; set; }
        public double? DelayMs { get; set; }
        public double Score { get; set; }
        public double LoopbackScore { get; set; }
        public int AmbiguousCandidateCount { get; set; }
        public bool TimestampError { get; set; }
        public bool Discontinuity { get; set; }
        public double MaxPacketTimestampResidualMs { get; set; }
        public bool Valid { get; set; }
        public string Reason { get; set; }
    }

    public static class Measurement
    {
        private const ulong QpcPerSecond = 10000000UL;
        private const int ExpectedStimulusRate = 48000;
        private const int TimestampErrorFlag = 0x4;
        private const int DiscontinuityFlag = 0x1;
        private const int SilentFlag = 0x2;
        private const double PeakThreshold = 0.30;
        private const double ValidScoreThreshold = 0.30;
        private const double PairWindowSeconds = 1.0;
        private const double MinimumPeakSeparationSeconds = 0.35;
        private const double TimestampResidualWarningMs = 2.0;

        public static MeasurementResult Analyze(string directory)
        {
            if (String.IsNullOrEmpty(directory))
                throw new ArgumentException("A capture directory is required.", "directory");
            if (!Directory.Exists(directory))
                throw new DirectoryNotFoundException("Capture directory does not exist: " + directory);

            Dictionary<string, string> metadata = ReadMetadata(Path.Combine(directory, "metadata.txt"));
            RequireMetadata(metadata, "qpcUnits", "100ns");
            RequireMetadata(metadata, "packetHeader", "uint64 qpc100ns, uint32 frames, uint32 flags; followed by frames*blockAlign PCM bytes");
            string mode;
            metadata.TryGetValue("mode", out mode);
            int? expectedPulseCount = ExpectedPulseCountForMode(mode);
            StreamFormat loopFormat = ReadFormat(metadata, "loopback");
            StreamFormat micFormat = ReadFormat(metadata, "microphone");
            if (loopFormat.SampleRate != micFormat.SampleRate)
                throw new InvalidDataException("Unsupported capture: loopback and microphone sample rates differ (" + loopFormat.SampleRate + " Hz and " + micFormat.SampleRate + " Hz); this analyzer's waveform refinement requires equal-rate sample windows.");
            if (loopFormat.SampleRate != ExpectedStimulusRate)
                throw new InvalidDataException("Unsupported capture: the probe stimulus is 48 kHz, but the captured streams report " + loopFormat.SampleRate + " Hz; the reference chirp and capture timeline would not match at this rate.");

            CaptureStream loopback = ReadStream(Path.Combine(directory, "loopback.pkt"), loopFormat);
            CaptureStream microphone = ReadStream(Path.Combine(directory, "microphone.pkt"), micFormat);
            MeasurementResult result = new MeasurementResult();
            result.LoopbackSampleRate = loopFormat.SampleRate;
            result.MicrophoneSampleRate = micFormat.SampleRate;
            result.ExpectedPulseCount = expectedPulseCount;

            AddPacketWarnings(result, "loopback", loopback, loopFormat.SampleRate);
            AddPacketWarnings(result, "microphone", microphone, micFormat.SampleRate);

            double[] stimulus = CreateChirp(loopFormat.SampleRate);
            List<Peak> loopPeaks = FindPeaks(loopback.Samples, stimulus, loopFormat.SampleRate, PeakThreshold);
            List<Peak> micPeaks = FindPeaks(microphone.Samples, stimulus, micFormat.SampleRate, PeakThreshold);
            result.PulseCount = loopPeaks.Count;
            result.DetectedSourceCount = loopPeaks.Count;
            result.MicrophonePeakCount = micPeaks.Count;
            if (expectedPulseCount.HasValue && loopPeaks.Count != expectedPulseCount.Value)
                result.Warnings.Add("Detected source chirp count (" + loopPeaks.Count + ") differs from expected " + expectedPulseCount.Value + " for mode=" + (mode ?? "") + "; missing or extra source pulses are not inferred.");
            if (loopPeaks.Count == 0)
                result.Warnings.Add("No loopback chirp was detected; check the selected output device and probe playback.");
            if (micPeaks.Count == 0)
                result.Warnings.Add("No microphone chirp was detected; no reference acoustic delay can be measured.");

            HashSet<int> usedMicrophonePeaks = new HashSet<int>();
            ulong lastMatchedMicrophoneQpc = 0;
            bool hasLastMatchedMicrophoneQpc = false;
            int lowScoreCount = 0;
            int ambiguousCount = 0;
            int missingCount = 0;
            List<double> validDelays = new List<double>();

            for (int pulse = 0; pulse < loopPeaks.Count; pulse++)
            {
                Peak source = loopPeaks[pulse];
                SamplePosition sourcePosition = PositionAt(loopback, source.Index, loopFormat.SampleRate);
                MeasurementSample sample = new MeasurementSample();
                sample.PulseIndex = pulse + 1;
                sample.LoopbackFrameIndex = source.Index;
                sample.LoopbackQpc100ns = sourcePosition.Qpc100ns;
                sample.LoopbackScore = source.Score;
                sample.Score = 0.0;
                sample.Reason = String.Empty;

                List<Peak> candidates = new List<Peak>();
                for (int i = 0; i < micPeaks.Count; i++)
                {
                    Peak candidate = micPeaks[i];
                    if (usedMicrophonePeaks.Contains(candidate.Index))
                        continue;
                    ulong candidateQpc = PositionAt(microphone, candidate.Index, micFormat.SampleRate).Qpc100ns;
                    if (candidateQpc <= sourcePosition.Qpc100ns)
                        continue;
                    if (candidateQpc - sourcePosition.Qpc100ns >= (ulong)(PairWindowSeconds * QpcPerSecond))
                        continue;
                    if (hasLastMatchedMicrophoneQpc && candidateQpc <= lastMatchedMicrophoneQpc)
                        continue;
                    candidates.Add(candidate);
                }

                if (candidates.Count == 0)
                {
                    missingCount++;
                    sample.Reason = "No unused microphone chirp peak occurred after this pulse within the 1000 ms pairing window.";
                    AddSourceHealth(sample, loopback, source.Index, stimulus.Length, loopFormat.SampleRate);
                    result.Samples.Add(sample);
                    continue;
                }

                Peak selected = candidates[0];
                ulong selectedQpc = PositionAt(microphone, selected.Index, micFormat.SampleRate).Qpc100ns;
                usedMicrophonePeaks.Add(selected.Index);
                lastMatchedMicrophoneQpc = selectedQpc;
                hasLastMatchedMicrophoneQpc = true;
                result.MatchedCount++;
                sample.AmbiguousCandidateCount = candidates.Count;
                if (candidates.Count > 1)
                {
                    ambiguousCount++;
                    AddReason(sample, "Ambiguous microphone pairing: " + candidates.Count + " unused peaks are in the 0–1000 ms window; the earliest peak was selected for reporting.");
                }

                int templateLength = Math.Min(stimulus.Length, loopback.Samples.Length - source.Index);
                if (templateLength != stimulus.Length)
                {
                    AddReason(sample, "The loopback chirp is truncated at the end of the capture.");
                    AddSourceHealth(sample, loopback, source.Index, templateLength, loopFormat.SampleRate);
                    result.Samples.Add(sample);
                    continue;
                }
                double[] sourceWaveform = new double[templateLength];
                Array.Copy(loopback.Samples, source.Index, sourceWaveform, 0, templateLength);
                int searchStart = Math.Max(0, selected.Index - 48);
                int searchEnd = Math.Min(microphone.Samples.Length, selected.Index + templateLength + 48);
                int searchLength = searchEnd - searchStart;
                if (searchLength < templateLength)
                {
                    AddReason(sample, "The microphone match window is truncated at the end of the capture.");
                    AddSourceHealth(sample, loopback, source.Index, templateLength, loopFormat.SampleRate);
                    result.Samples.Add(sample);
                    continue;
                }
                double[] search = new double[searchLength];
                Array.Copy(microphone.Samples, searchStart, search, 0, searchLength);
                Peak match = BestMatch(search, sourceWaveform);
                if (match.Index < 0)
                {
                    AddReason(sample, "No valid normalized correlation could be calculated for the microphone match window.");
                    AddSourceHealth(sample, loopback, source.Index, templateLength, loopFormat.SampleRate);
                    result.Samples.Add(sample);
                    continue;
                }

                int microphoneIndex = searchStart + match.Index;
                SamplePosition microphonePosition = PositionAt(microphone, microphoneIndex, micFormat.SampleRate);
                sample.MicrophoneFrameIndex = microphoneIndex;
                sample.MicrophoneQpc100ns = microphonePosition.Qpc100ns;
                sample.Score = match.Score;
                sample.DelayMs = ((double)microphonePosition.Qpc100ns - (double)sourcePosition.Qpc100ns) / 10000.0;
                if (sample.Score < ValidScoreThreshold)
                {
                    lowScoreCount++;
                    AddReason(sample, "Low NCC score " + sample.Score.ToString("F3", CultureInfo.InvariantCulture) + "; minimum accepted score is " + ValidScoreThreshold.ToString("F2", CultureInfo.InvariantCulture) + ".");
                }

                AddSourceHealth(sample, loopback, source.Index, templateLength, loopFormat.SampleRate);
                AddMicrophoneHealth(sample, microphone, microphoneIndex, templateLength, micFormat.SampleRate);
                if (sample.DelayMs.Value <= 0.0 || sample.DelayMs.Value >= PairWindowSeconds * 1000.0)
                    AddReason(sample, "Refined microphone delay is outside the 0–1000 ms pairing window.");
                sample.Valid = sample.Reason.Length == 0;
                if (sample.Valid)
                    validDelays.Add(sample.DelayMs.Value);
                result.Samples.Add(sample);
            }

            if (micPeaks.Count > usedMicrophonePeaks.Count)
                result.Warnings.Add((micPeaks.Count - usedMicrophonePeaks.Count) + " microphone chirp peak(s) were not paired to a loopback pulse.");
            if (missingCount > 0)
                result.Warnings.Add(missingCount + " loopback pulse(s) have no microphone match.");
            if (ambiguousCount > 0)
                result.Warnings.Add(ambiguousCount + " loopback pulse(s) had ambiguous microphone candidates and are invalid.");
            if (lowScoreCount > 0)
                result.Warnings.Add(lowScoreCount + " microphone match(es) had a low NCC score and are invalid.");

            result.ValidCount = validDelays.Count;
            SetStatistics(result, validDelays);
            if (validDelays.Count == 0)
                result.Warnings.Add("No valid paired pulse remains after correlation and capture-integrity checks; summary statistics are unavailable.");
            return result;
        }

        private static Dictionary<string, string> ReadMetadata(string path)
        {
            if (!File.Exists(path))
                throw new FileNotFoundException("Capture metadata.txt was not found.", path);
            Dictionary<string, string> values = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
            using (StreamReader reader = new StreamReader(path))
            {
                string line;
                while ((line = reader.ReadLine()) != null)
                {
                    int separator = line.IndexOf('=');
                    if (separator <= 0)
                        continue;
                    string key = line.Substring(0, separator).Trim();
                    string value = line.Substring(separator + 1).Trim();
                    if (values.ContainsKey(key))
                        throw new InvalidDataException("Duplicate metadata key: " + key);
                    values.Add(key, value);
                }
            }
            return values;
        }

        private static void RequireMetadata(Dictionary<string, string> metadata, string key, string expected)
        {
            string value;
            if (!metadata.TryGetValue(key, out value) || !String.Equals(value, expected, StringComparison.OrdinalIgnoreCase))
                throw new InvalidDataException("Unsupported or missing metadata " + key + "; expected " + expected + ".");
        }

        private static int? ExpectedPulseCountForMode(string mode)
        {
            if (String.Equals(mode, "formal", StringComparison.OrdinalIgnoreCase))
                return 10;
            if (String.Equals(mode, "precheck", StringComparison.OrdinalIgnoreCase))
                return 2;
            return null;
        }

        private static StreamFormat ReadFormat(Dictionary<string, string> metadata, string name)
        {
            int sampleRate = RequiredInt(metadata, name + ".sampleRate");
            int channels = RequiredInt(metadata, name + ".channels");
            int blockAlign = RequiredInt(metadata, name + ".blockAlign");
            int bitsPerSample = RequiredInt(metadata, name + ".bitsPerSample");
            string subFormat;
            if (!metadata.TryGetValue(name + ".subFormat", out subFormat) || !subFormat.StartsWith("00000003-", StringComparison.OrdinalIgnoreCase) || bitsPerSample != 32)
                throw new InvalidDataException("Unsupported " + name + " PCM format; expected IEEE float32 (subFormat 00000003-..., bitsPerSample=32).");
            if (sampleRate <= 0 || channels <= 0 || channels > 64)
                throw new InvalidDataException("Invalid " + name + " sample rate or channel count in metadata.");
            if (sampleRate < 17000)
                throw new InvalidDataException("The " + name + " sample rate is below 17 kHz, which cannot represent the probe chirp up to 8.5 kHz.");
            if (blockAlign != checked(channels * 4))
                throw new InvalidDataException("Unsupported " + name + " blockAlign; IEEE float32 requires channels × 4 bytes.");
            return new StreamFormat(sampleRate, channels, blockAlign);
        }

        private static int RequiredInt(Dictionary<string, string> metadata, string key)
        {
            string text;
            int value;
            if (!metadata.TryGetValue(key, out text) || !Int32.TryParse(text, NumberStyles.Integer, CultureInfo.InvariantCulture, out value))
                throw new InvalidDataException("Missing or invalid integer metadata: " + key);
            return value;
        }

        private static CaptureStream ReadStream(string path, StreamFormat format)
        {
            if (!File.Exists(path))
                throw new FileNotFoundException("Capture packet file was not found.", path);
            List<double> samples = new List<double>();
            List<Packet> packets = new List<Packet>();
            byte[] header = new byte[16];
            using (FileStream stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read))
            {
                while (true)
                {
                    int first = stream.ReadByte();
                    if (first < 0)
                        break;
                    header[0] = (byte)first;
                    ReadExactly(stream, header, 1, 15, "truncated packet header");
                    ulong qpc = ReadUInt64LittleEndian(header, 0);
                    uint frames = ReadUInt32LittleEndian(header, 8);
                    uint flags = ReadUInt32LittleEndian(header, 12);
                    if (frames == 0)
                        throw new InvalidDataException("A packet contains zero frames: " + path);
                    long payloadLength = checked((long)frames * (long)format.BlockAlign);
                    if (payloadLength > Int32.MaxValue)
                        throw new InvalidDataException("A packet payload is too large to read safely: " + path);
                    if ((long)samples.Count + frames > Int32.MaxValue)
                        throw new InvalidDataException("The capture contains more frames than this .NET analysis can index safely.");
                    byte[] payload = new byte[(int)payloadLength];
                    ReadExactly(stream, payload, 0, payload.Length, "truncated PCM packet payload");
                    int start = samples.Count;
                    if ((flags & SilentFlag) != 0)
                    {
                        for (int frame = 0; frame < (int)frames; frame++)
                            samples.Add(0.0);
                    }
                    else
                    {
                        for (int frame = 0; frame < (int)frames; frame++)
                        {
                            double sum = 0.0;
                            int frameByte = frame * format.BlockAlign;
                            for (int channel = 0; channel < format.Channels; channel++)
                            {
                                float value = ReadSingleLittleEndian(payload, frameByte + channel * 4);
                                if (Single.IsNaN(value) || Single.IsInfinity(value))
                                    throw new InvalidDataException("Non-finite IEEE float sample in " + path + ".");
                                sum += value;
                            }
                            samples.Add(sum / format.Channels);
                        }
                    }
                    packets.Add(new Packet(qpc, frames, flags, start));
                }
            }
            return new CaptureStream(samples.ToArray(), packets);
        }

        private static void ReadExactly(Stream stream, byte[] buffer, int offset, int count, string error)
        {
            while (count > 0)
            {
                int read = stream.Read(buffer, offset, count);
                if (read <= 0)
                    throw new InvalidDataException(error + ".");
                offset += read;
                count -= read;
            }
        }

        private static ulong ReadUInt64LittleEndian(byte[] bytes, int offset)
        {
            return (ulong)bytes[offset] | ((ulong)bytes[offset + 1] << 8) | ((ulong)bytes[offset + 2] << 16) | ((ulong)bytes[offset + 3] << 24) |
                ((ulong)bytes[offset + 4] << 32) | ((ulong)bytes[offset + 5] << 40) | ((ulong)bytes[offset + 6] << 48) | ((ulong)bytes[offset + 7] << 56);
        }

        private static uint ReadUInt32LittleEndian(byte[] bytes, int offset)
        {
            return (uint)(bytes[offset] | (bytes[offset + 1] << 8) | (bytes[offset + 2] << 16) | (bytes[offset + 3] << 24));
        }

        private static float ReadSingleLittleEndian(byte[] bytes, int offset)
        {
            if (BitConverter.IsLittleEndian)
                return BitConverter.ToSingle(bytes, offset);
            byte[] raw = new byte[4];
            Buffer.BlockCopy(bytes, offset, raw, 0, 4);
            Array.Reverse(raw);
            return BitConverter.ToSingle(raw, 0);
        }

        private static double[] CreateChirp(int sampleRate)
        {
            int length = sampleRate / 10;
            double[] chirp = new double[length];
            double seconds = (double)length / sampleRate;
            for (int i = 0; i < length; i++)
            {
                double time = (double)i / sampleRate;
                double phase = 2.0 * Math.PI * (700.0 * time + (8500.0 - 700.0) * time * time / (2.0 * seconds));
                double window = 0.5 - 0.5 * Math.Cos(2.0 * Math.PI * time / seconds);
                chirp[i] = window * Math.Sin(phase);
            }
            return chirp;
        }

        private static List<Peak> FindPeaks(double[] data, double[] template, int sampleRate, double minimumScore)
        {
            List<Peak> candidates = Correlate(data, template, minimumScore, true);
            candidates.Sort(delegate(Peak left, Peak right) { return right.Score.CompareTo(left.Score); });
            List<Peak> selected = new List<Peak>();
            int separation = (int)(sampleRate * MinimumPeakSeparationSeconds);
            for (int i = 0; i < candidates.Count; i++)
            {
                Peak candidate = candidates[i];
                bool farEnough = true;
                for (int j = 0; j < selected.Count; j++)
                {
                    if (Math.Abs((long)candidate.Index - selected[j].Index) < separation)
                    {
                        farEnough = false;
                        break;
                    }
                }
                if (farEnough)
                    selected.Add(candidate);
            }
            selected.Sort(delegate(Peak left, Peak right) { return left.Index.CompareTo(right.Index); });
            return selected;
        }

        private static Peak BestMatch(double[] data, double[] template)
        {
            List<Peak> values = Correlate(data, template, Double.NegativeInfinity, false);
            if (values.Count == 0)
                return new Peak(-1, 0.0);
            return values[0];
        }

        private static List<Peak> Correlate(double[] data, double[] template, double minimumScore, bool localMaximaOnly)
        {
            List<Peak> result = new List<Peak>();
            int n = data.Length;
            int m = template.Length;
            if (m == 0 || n < m)
                return result;
            int convolutionLength = checked(n + m - 1);
            int fftSize = NextPowerOfTwo(convolutionLength);
            Complex[] signal = new Complex[fftSize];
            Complex[] reversedTemplate = new Complex[fftSize];
            double templateEnergy = 0.0;
            for (int i = 0; i < n; i++)
                signal[i] = new Complex(data[i], 0.0);
            for (int i = 0; i < m; i++)
            {
                double value = template[m - 1 - i];
                reversedTemplate[i] = new Complex(value, 0.0);
                templateEnergy += value * value;
            }
            if (templateEnergy <= 1e-24)
                return result;
            Fft.Transform(signal, false);
            Fft.Transform(reversedTemplate, false);
            for (int i = 0; i < fftSize; i++)
                signal[i] *= reversedTemplate[i];
            Fft.Transform(signal, true);

            double windowEnergy = 0.0;
            for (int i = 0; i < m; i++)
                windowEnergy += data[i] * data[i];
            double previousPrevious = Double.NaN;
            double previous = Double.NaN;
            double bestScore = Double.NegativeInfinity;
            int bestIndex = -1;
            int count = n - m + 1;
            for (int start = 0; start < count; start++)
            {
                if (start > 0)
                    windowEnergy += data[start + m - 1] * data[start + m - 1] - data[start - 1] * data[start - 1];
                double denominator = Math.Sqrt(Math.Max(0.0, windowEnergy) * templateEnergy);
                double next = denominator > 1e-12 ? signal[start + m - 1].Real / denominator : 0.0;
                if (next > 1.0) next = 1.0;
                else if (next < -1.0) next = -1.0;
                if (!localMaximaOnly)
                {
                    if (next > bestScore)
                    {
                        bestScore = next;
                        bestIndex = start;
                    }
                }
                else if (start >= 2 && previous > previousPrevious && previous >= next && previous >= minimumScore)
                {
                    result.Add(new Peak(start - 1, previous));
                }
                previousPrevious = previous;
                previous = next;
            }
            if (!localMaximaOnly && bestIndex >= 0)
                result.Add(new Peak(bestIndex, bestScore));
            return result;
        }

        private static int NextPowerOfTwo(int value)
        {
            if (value <= 0 || value > (1 << 29))
                throw new InvalidDataException("Capture is too large for the FFT correlation workspace.");
            int result = 1;
            while (result < value)
                result = checked(result << 1);
            return result;
        }

        private static SamplePosition PositionAt(CaptureStream stream, int index, int sampleRate)
        {
            int low = 0;
            int high = stream.Packets.Count - 1;
            while (low <= high)
            {
                int middle = low + (high - low) / 2;
                Packet packet = stream.Packets[middle];
                if (index < packet.StartFrame)
                    high = middle - 1;
                else if ((long)index >= (long)packet.StartFrame + packet.Frames)
                    low = middle + 1;
                else
                {
                    int offset = index - packet.StartFrame;
                    ulong delta = (ulong)Math.Round((double)offset * QpcPerSecond / sampleRate, MidpointRounding.ToEven);
                    return new SamplePosition(checked(packet.Qpc100ns + delta), middle, offset);
                }
            }
            throw new InvalidDataException("A detected sample index did not map to a packet timestamp.");
        }

        private static void AddPacketWarnings(MeasurementResult result, string name, CaptureStream stream, int sampleRate)
        {
            int timestampErrors = 0;
            int discontinuities = 0;
            int residualErrors = 0;
            double maximumResidual = 0.0;
            for (int i = 0; i < stream.Packets.Count; i++)
            {
                Packet packet = stream.Packets[i];
                if ((packet.Flags & TimestampErrorFlag) != 0) timestampErrors++;
                if ((packet.Flags & DiscontinuityFlag) != 0) discontinuities++;
                if (i == 0) continue;
                Packet previous = stream.Packets[i - 1];
                double expected = (double)previous.Frames * QpcPerSecond / sampleRate;
                double residual = (double)packet.Qpc100ns - previous.Qpc100ns - expected;
                double absoluteMs = Math.Abs(residual) / 10000.0;
                if (absoluteMs > maximumResidual) maximumResidual = absoluteMs;
                if (absoluteMs > TimestampResidualWarningMs) residualErrors++;
            }
            if (timestampErrors > 0)
                result.Warnings.Add(name + " has " + timestampErrors + " packet(s) with the WASAPI timestamp-error flag.");
            if (discontinuities > 0)
                result.Warnings.Add(name + " has " + discontinuities + " packet(s) with the WASAPI data-discontinuity flag.");
            if (residualErrors > 0)
                result.Warnings.Add(name + " has " + residualErrors + " adjacent packet QPC interval(s) with a residual over " + TimestampResidualWarningMs.ToString("F1", CultureInfo.InvariantCulture) + " ms (maximum " + maximumResidual.ToString("F3", CultureInfo.InvariantCulture) + " ms).");
        }

        private static void AddSourceHealth(MeasurementSample sample, CaptureStream stream, int index, int length, int sampleRate)
        {
            AddSegmentHealth(sample, "loopback", stream, index, length, sampleRate);
        }

        private static void AddMicrophoneHealth(MeasurementSample sample, CaptureStream stream, int index, int length, int sampleRate)
        {
            AddSegmentHealth(sample, "microphone", stream, index, length, sampleRate);
        }

        private static void AddSegmentHealth(MeasurementSample sample, string name, CaptureStream stream, int index, int length, int sampleRate)
        {
            if (length <= 0)
                return;
            int end = index + length;
            int firstPacket = FindPacketIndex(stream.Packets, index);
            int lastPacket = FindPacketIndex(stream.Packets, end - 1);
            for (int i = firstPacket; i <= lastPacket; i++)
            {
                Packet packet = stream.Packets[i];
                if ((packet.Flags & TimestampErrorFlag) != 0)
                {
                    sample.TimestampError = true;
                    AddReason(sample, name + " segment contains a WASAPI timestamp-error packet.");
                }
                if ((packet.Flags & DiscontinuityFlag) != 0)
                {
                    sample.Discontinuity = true;
                    AddReason(sample, name + " segment contains a WASAPI data-discontinuity packet.");
                }
                if (i > firstPacket)
                {
                    Packet previous = stream.Packets[i - 1];
                    double expected = (double)previous.Frames * QpcPerSecond / sampleRate;
                    double residualMs = Math.Abs(((double)packet.Qpc100ns - previous.Qpc100ns - expected) / 10000.0);
                    if (residualMs > sample.MaxPacketTimestampResidualMs)
                        sample.MaxPacketTimestampResidualMs = residualMs;
                    if (residualMs > TimestampResidualWarningMs)
                        AddReason(sample, name + " packet QPC interval residual is " + residualMs.ToString("F3", CultureInfo.InvariantCulture) + " ms (> " + TimestampResidualWarningMs.ToString("F1", CultureInfo.InvariantCulture) + " ms).");
                }
            }
        }

        private static int FindPacketIndex(List<Packet> packets, int frame)
        {
            int low = 0;
            int high = packets.Count - 1;
            while (low <= high)
            {
                int middle = low + (high - low) / 2;
                Packet packet = packets[middle];
                if (frame < packet.StartFrame)
                    high = middle - 1;
                else if ((long)frame >= (long)packet.StartFrame + packet.Frames)
                    low = middle + 1;
                else
                    return middle;
            }
            throw new InvalidDataException("A segment sample index did not map to a packet.");
        }

        private static void AddReason(MeasurementSample sample, string reason)
        {
            if (sample.Reason.Length > 0)
                sample.Reason += " ";
            sample.Reason += reason;
        }

        private static void SetStatistics(MeasurementResult result, List<double> delays)
        {
            if (delays.Count == 0)
                return;
            delays.Sort();
            result.MinMs = delays[0];
            result.MaxMs = delays[delays.Count - 1];
            result.MedianMs = Percentile(delays, 0.50);
            result.P95Ms = Percentile(delays, 0.95);
        }

        private static double Percentile(List<double> sorted, double percentile)
        {
            double position = (sorted.Count - 1) * percentile;
            int lower = (int)Math.Floor(position);
            int upper = (int)Math.Ceiling(position);
            if (lower == upper)
                return sorted[lower];
            return sorted[lower] + (sorted[upper] - sorted[lower]) * (position - lower);
        }

        private sealed class StreamFormat
        {
            public readonly int SampleRate;
            public readonly int Channels;
            public readonly int BlockAlign;
            public StreamFormat(int sampleRate, int channels, int blockAlign) { SampleRate = sampleRate; Channels = channels; BlockAlign = blockAlign; }
        }

        private sealed class CaptureStream
        {
            public readonly double[] Samples;
            public readonly List<Packet> Packets;
            public CaptureStream(double[] samples, List<Packet> packets) { Samples = samples; Packets = packets; }
        }

        private sealed class Packet
        {
            public readonly ulong Qpc100ns;
            public readonly int Frames;
            public readonly uint Flags;
            public readonly int StartFrame;
            public Packet(ulong qpc100ns, uint frames, uint flags, int startFrame) { Qpc100ns = qpc100ns; Frames = checked((int)frames); Flags = flags; StartFrame = startFrame; }
        }

        private sealed class SamplePosition
        {
            public readonly ulong Qpc100ns;
            public readonly int PacketIndex;
            public readonly int Offset;
            public SamplePosition(ulong qpc100ns, int packetIndex, int offset) { Qpc100ns = qpc100ns; PacketIndex = packetIndex; Offset = offset; }
        }

        private sealed class Peak
        {
            public readonly int Index;
            public readonly double Score;
            public Peak(int index, double score) { Index = index; Score = score; }
        }

        private static class Fft
        {
            public static void Transform(Complex[] values, bool inverse)
            {
                int n = values.Length;
                for (int i = 1, j = 0; i < n; i++)
                {
                    int bit = n >> 1;
                    for (; (j & bit) != 0; bit >>= 1)
                        j ^= bit;
                    j ^= bit;
                    if (i < j)
                    {
                        Complex swap = values[i];
                        values[i] = values[j];
                        values[j] = swap;
                    }
                }
                for (int length = 2; length <= n; length <<= 1)
                {
                    double angle = (inverse ? 2.0 : -2.0) * Math.PI / length;
                    Complex root = new Complex(Math.Cos(angle), Math.Sin(angle));
                    int half = length >> 1;
                    for (int start = 0; start < n; start += length)
                    {
                        Complex factor = Complex.One;
                        for (int offset = 0; offset < half; offset++)
                        {
                            Complex even = values[start + offset];
                            Complex odd = values[start + offset + half] * factor;
                            values[start + offset] = even + odd;
                            values[start + offset + half] = even - odd;
                            factor *= root;
                        }
                    }
                    if (length == n)
                        break;
                }
                if (inverse)
                    for (int i = 0; i < n; i++)
                        values[i] /= n;
            }
        }
    }
}
