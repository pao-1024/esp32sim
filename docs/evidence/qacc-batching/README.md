# EX238: native QACC batching

Related: EX126 proposed native SIMD. This change uses scalar Rust with constant lane widths,
keeps both 160-bit accumulator halves local for the entire instruction, and predecodes
VSMULAS operands. It does not substitute a neural-network implementation or change guest timing.

Baseline: `507809fe54758b6cc2890e6a787419c9fa44809e`.
Candidate `xtensa-lx7/src/pie.rs` SHA256:
`17cac39036b1bcb20234ac31eebbba96690a2aa4d992d7dcb4aa51266334ec33`.
Workload: PAO formal ESP32-S3 firmware, 240 MHz, 16 MB flash, 8 MB PSRAM, native AArch64
release build, Rust 1.99.1. ELF SHA256:
`90198d93ed7f23200bacf6b67bf27bc1040af557f0261100e3f6db0c8f2b42b0`.
Mono PCM16 16 kHz input: 111688 bytes, SHA256
`4fb6a57eeefe34373f2667458e38d89d760a93b2fa81fb653d19f009bee030b6`.
The private device flash and recording are not retained here; this limits exact reproduction.
The timed scripted input is unscaled: PAO's codec shifts its 32-bit input by 12, so samples
above 2047 clip. This is a compute-path stress input, not a speech-recognition quality test.

Run both binaries with the same flash snapshot, ROM, ELF and flash segments, `--max-lag 4`,
`--max-seconds 20`, and a script containing `6 i2spcm 1 <input.pcm> 16000`, repeated at 10 and
14 seconds. No profiling in timed runs. Capture child user CPU time and elapsed wall time.

| Alternating pair | Baseline user CPU / wall (s) | Candidate user CPU / wall (s) |
| --- | --- | --- |
| 1 | 17.258245 / 21.400 | 12.277668 / 20.442 |
| 2 | 16.621095 / 20.254 | 12.439939 / 20.744 |

Median user CPU falls 27.0%. Wall time includes real-time pacing, startup and live network I/O;
these are not deterministic instruction-matched benchmarks. Baseline totals were 929506091 and
909511910 instructions; candidate totals 909976112 and 910180151. Baseline-to-baseline variation
is larger than candidate-to-baseline variation in pair 2. Do not infer a guest semantic change
or a guaranteed 1x speech speed from these counts or averages.

A separate block profile attributes 223652502 instructions (24.24% of 922785408) to
`dl_tie728_pointwise_conv1d_qacc_i16o16k8b16_preload_core`; the disassembly uses VSMULAS.QACC
and VSMULAS.QACC.LD.INCP. Another 6.12% is doubleswish. The optimization targets this measured
convolution path. Real-time PCM is separately buffered in order without discarding old frames;
the timing input uses the scripted I2S source, so changing live buffering does not explain it.

Correctness checks: packed/table execution comparison includes QACC, GPR/Q-register aliases,
CP-disabled exceptions, load faults and bulk/non-bulk buses. Sixteen thousand seeded cases
compare batched signed/unsigned 8/16-bit MACs with the original scalar per-lane implementation,
including saturation and extreme operands. Guest arithmetic, instruction counts per operation,
Q readiness and exception ordering remain the contract.

Separate live correctness run: mono PCM sent through WebSocket type 5 at 16 kHz, scaled by
1/16 to match PAO's browser worklet and codec slot format. A synthesized configured wake phrase
triggered MultiNet, connected to the existing service and produced ASR text, TTS and I2S output.
The 50-second host observation advanced 49.98 guest seconds in 49.98 wall seconds; minimum
reported rolling speed was 0.850x, final speed 0.998x, final lag zero, no microphone errors.
Wake detection to listening: approximately 0.31 wall seconds; WebSocket handshake: 0.20 s;
ASR text to first response: 1.11 s. These service times are one observation, not an SLA.
The prompt audio sent during TTS was not recognized as a second question in this run; a
separate BOOT/listening input produced the expected question transcription and reply.

## General firmware comparison

Against organization `main` at `b97fdc7c3d807884a2e2ee02a5b499559e854e31`, three alternating
pairs per workload, native release binaries, 30 guest seconds, matching ROM and manifest images
from `web/wasm/fw/{hello,c3-hello,c6-hello,pocket-tank}.json`; Pocket Tank also loads its model at
0x290000. No audio input or browser. Instruction totals match in every pair.

| Workload | Instructions | Main median user (s) | Candidate median user (s) |
| --- | --- | --- | --- |
| S3 hello | 18788848 | 0.291119 | 0.308560 |
| C3 hello | 4800000000 | 2.654076 | 2.637855 |
| C6 hello | 4800000000 | 1.748182 | 1.782078 |
| Pocket Tank | 10656135661 | 43.739595 | 43.713145 |

S3 hello initially regressed 6.0% (17 ms); seven additional alternating pairs after builds and
the other benchmarks finished gave 0.285402 → 0.289055 s (+1.3%, 4 ms), with overlapping ranges:
main 0.279766–0.302729, candidate 0.284585–0.299665. This small workload does not establish zero
overhead. C6 is +1.9%, C3 −0.6%, Pocket Tank −0.06%. These runs compare the entire PR tip with
main, including RMT and audio changes, not only QACC. The compute-path speech improvement is
measured separately above against the preceding PR tip.
