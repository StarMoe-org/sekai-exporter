#!/usr/bin/env node
// Compares two forms of the Live2D motion clips in object documents: the clip's own curves,
// evaluated as Unity stores them, and the same clip written as motion3.json and evaluated by
// Live2D's Cubism Web Framework. See README.md.
//
// usage: node compare.mjs <framework directory> <objects directory>... [--report <file>]

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import * as esbuild from 'esbuild';

const f32 = Math.fround;
const DT = f32(1 / 60);
const THRESHOLDS = [1e-6, 1e-5, 1e-4, 1e-3, 1e-2, 1e-1];
/** The largest difference the exact motion3, read at 32-bit precision, may show. */
const TOLERANCE = 1e-4;
const ANIMATION_CLIP = 74;
const MONO_BEHAVIOUR = 114;
const META_SCRIPT = 'Live2DBuildMotionMetaData';

// ---------------------------------------------------------------------------------------------
// The framework, bundled from a checkout. Its only change is that `evaluateCurve`, the function
// `CubismMotion` evaluates every curve with, is exported.

async function loadFramework(dir) {
  const src = path.resolve(dir, 'src');
  const entry = [
    `export { CubismFramework, Option, LogLevel } from ${JSON.stringify(path.join(src, 'live2dcubismframework.ts'))};`,
    `export { CubismMotion, evaluateCurve } from ${JSON.stringify(path.join(src, 'motion', 'cubismmotion.ts'))};`,
  ].join('\n');
  const exportEvaluator = {
    name: 'export-evaluate-curve',
    setup(build) {
      build.onLoad({ filter: /[\\/]motion[\\/]cubismmotion\.ts$/ }, async (args) => ({
        contents: `${await fs.promises.readFile(args.path, 'utf8')}\nexport { evaluateCurve };\n`,
        loader: 'ts',
      }));
    },
  };
  const result = await esbuild.build({
    stdin: { contents: entry, resolveDir: src, loader: 'ts' },
    bundle: true,
    format: 'esm',
    platform: 'node',
    write: false,
    plugins: [exportEvaluator],
    logLevel: 'error',
  });
  const file = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'cubism-')), 'framework.mjs');
  fs.writeFileSync(file, result.outputFiles[0].contents);
  // Motions need no Cubism Core; the framework only calls these while starting up.
  globalThis.Live2DCubismCore = {
    Logging: { csmSetLogFunction() {} },
    Version: { csmGetVersion: () => 0 },
    Memory: { initializeAmountOfMemory() {} },
  };
  const fw = await import(pathToFileURL(file).href);
  const option = new fw.Option();
  option.logFunction = () => {};
  option.loggingLevel = fw.LogLevel.LogLevel_Off;
  fw.CubismFramework.startUp(option);
  fw.CubismFramework.initialize();
  return fw;
}

// ---------------------------------------------------------------------------------------------
// Clips from object documents.

function* objectDocuments(dir) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) yield* objectDocuments(full);
    else if (entry.name === '_objects.json') yield full;
  }
}

const stem = (container) => container?.replace(/\.[^./]*$/, '') ?? null;

function* clips(file) {
  const doc = JSON.parse(fs.readFileSync(file, 'utf8'));
  for (const serialized of doc.files) {
    const metas = new Map();
    for (const o of serialized.objects) {
      if (o.class_id === MONO_BEHAVIOUR && o.fields && String(o.script ?? '').endsWith(META_SCRIPT)) {
        metas.set(stem(o.container?.[0]), o.fields);
      }
    }
    for (const o of serialized.objects) {
      if (o.class_id === ANIMATION_CLIP && o.fields) {
        yield { bundle: doc.bundle, name: o.fields.m_Name, fields: o.fields, meta: metas.get(stem(o.container?.[0])) };
      }
    }
  }
}

const number = (v) => (typeof v === 'string' ? Number(v) : v);

/** `m_StreamedClip.data`: per frame `[time f32][key count i32]`, then per key `[curve i32][a b c d f32]`. */
function streamedSegments(words, curveCount) {
  const view = new DataView(new ArrayBuffer(words.length * 4));
  words.forEach((w, i) => view.setUint32(i * 4, w, true));
  const curves = Array.from({ length: curveCount }, () => []);
  let i = 0;
  while (i < words.length) {
    const t = view.getFloat32(i * 4, true);
    const count = view.getInt32((i + 1) * 4, true);
    i += 2;
    for (let k = 0; k < count; k++, i += 5) {
      const index = view.getInt32(i * 4, true);
      const coeff = [1, 2, 3, 4].map((j) => view.getFloat32((i + j) * 4, true));
      curves[index].push({ t, coeff });
    }
  }
  return curves;
}

/** The curves of a clip in Unity's order: streamed, dense, constant. */
function clipCurves(fields) {
  const muscle = fields.m_MuscleClip;
  const data = muscle.m_Clip.data;
  const streamed = data.m_StreamedClip;
  const curves = streamedSegments(streamed.data, streamed.curveCount).map((segments) => ({ kind: 'streamed', segments }));
  const dense = data.m_DenseClip;
  for (let j = 0; j < dense.m_CurveCount; j++) {
    const samples = [];
    for (let k = 0; k < dense.m_FrameCount; k++) samples.push(f32(number(dense.m_SampleArray[k * dense.m_CurveCount + j])));
    curves.push({ kind: 'dense', begin: f32(number(dense.m_BeginTime)), rate: f32(number(dense.m_SampleRate)), samples });
  }
  for (const value of data.m_ConstantClip.data) curves.push({ kind: 'constant', value: f32(number(value)) });
  return curves;
}

// ---------------------------------------------------------------------------------------------
// The clip's curves evaluated as Unity stores them, in 32-bit arithmetic.

function partition(segments, time) {
  let lo = 0;
  let hi = segments.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (segments[mid].t <= time) lo = mid + 1;
    else hi = mid;
  }
  return Math.max(lo - 1, 0);
}

function evaluateUnity(curve, time) {
  switch (curve.kind) {
    case 'constant':
      return curve.value;
    case 'streamed': {
      if (curve.segments.length === 0) return 0;
      const { t, coeff: [a, b, c, d] } = curve.segments[partition(curve.segments, time)];
      const dt = f32(time - t);
      return f32(f32(f32(f32(f32(f32(a * dt) + b) * dt) + c) * dt) + d);
    }
    case 'dense': {
      const s = curve.samples;
      if (s.length === 0) return 0;
      const position = Math.max(f32(f32(time - curve.begin) * curve.rate), 0);
      const last = s.length - 1;
      const lower = Math.min(Math.floor(position), last);
      const upper = Math.min(lower + 1, last);
      const fraction = Math.min(Math.max(f32(position - lower), 0), 1);
      return f32(s[lower] + f32(f32(s[upper] - s[lower]) * fraction));
    }
    default:
      throw new Error(curve.kind);
  }
}

/** The viewer's clock: 1/60 s steps summed in 32 bits, held at the end; then every midpoint. */
function sampleTimes(length) {
  const frames = [];
  let t = 0;
  for (;;) {
    frames.push(Math.min(t, length));
    if (t > length) break;
    t = f32(t + DT);
  }
  const mids = [];
  for (let i = 0; i + 1 < frames.length; i++) mids.push(f32(frames[i] + f32(f32(frames[i + 1] - frames[i]) / 2)));
  return { frames, mids };
}

// ---------------------------------------------------------------------------------------------
// The clip as motion3.json: every interval between the clip's keys becomes one segment. A cubic
// becomes the restricted Bezier with control points at a third of the interval, which describes
// the same cubic; a constant piece followed by a jump becomes a stepped segment; dense samples
// become linear segments. This inverts the mapping Cubism SDK for Unity applies on import.

function cubic([a, b, c, d], dt) {
  return ((a * dt + b) * dt + c) * dt + d;
}

function slope([a, b, c], dt) {
  return (3 * a * dt + 2 * b) * dt + c;
}

/**
 * Segments of one curve over [0, length], and the largest jump between a cubic's end and the next
 * key's value: a segment shares its end point with the next, so a jump is not representable.
 */
function motion3Segments(curve, start, length) {
  if (curve.kind === 'constant') return { points: [0, curve.value, 0, length, curve.value], jump: 0 };
  if (curve.kind === 'dense') {
    const points = [];
    curve.samples.forEach((v, k) => {
      const t = f32(curve.begin + f32(k / curve.rate)) - start;
      points.push(...(k === 0 ? [t, v] : [0, t, v]));
    });
    return { points, jump: 0 };
  }
  const segs = curve.segments;
  const end = start + length;
  const starts = [...new Set(segs.map((s) => s.t).filter((t) => t > start && t < end))];
  const bounds = [start, ...starts, end];
  const points = [0, evaluateUnity(curve, f32(start))];
  let jump = 0;
  for (let k = 0; k + 1 < bounds.length; k++) {
    const [t0, t1] = [bounds[k], bounds[k + 1]];
    const seg = segs[partition(segs, t0)];
    const [a, b, c] = seg.coeff;
    const h = t1 - t0;
    const x0 = t0 - seg.t;
    const v0 = cubic(seg.coeff, x0);
    const v1 = cubic(seg.coeff, x0 + h);
    const next = k + 2 < bounds.length ? evaluateUnity(curve, f32(t1)) : v1;
    if (a === 0 && b === 0 && c === 0) {
      // Constant until the next key: a stepped segment ends on the next key's value.
      points.push(2, t1 - start, next);
      continue;
    }
    jump = Math.max(jump, Math.abs(next - v1));
    points.push(
      1,
      t0 - start + h / 3, v0 + (slope(seg.coeff, x0) * h) / 3,
      t1 - start - h / 3, v1 - (slope(seg.coeff, x0 + h) * h) / 3,
      t1 - start, v1,
    );
  }
  return { points, jump };
}

function motion3(clip, curves, round) {
  const muscle = clip.fields.m_MuscleClip;
  const start = f32(number(muscle.m_StartTime));
  const length = f32(f32(number(muscle.m_StopTime)) - start);
  const write = round ? (v) => Number(v.toFixed(3)) : (v) => v;
  let segments = 0;
  let pointCount = 0;
  let jump = 0;
  const out = curves.map((curve, index) => {
    const built = motion3Segments(curve, start, length);
    jump = Math.max(jump, built.jump);
    const p = built.points;
    pointCount += 1;
    for (let i = 2; i < p.length;) {
      const bezier = p[i] === 1;
      segments += 1;
      pointCount += bezier ? 3 : 1;
      i += bezier ? 7 : 3;
    }
    return { Target: 'Parameter', Id: `curve${index}`, Segments: p.map(write) };
  });
  const events = clip.fields.m_Events ?? [];
  const meta = clip.meta;
  return {
    json: {
      Version: 3,
      Meta: {
        Duration: length,
        Fps: number(clip.fields.m_SampleRate),
        Loop: Boolean(muscle.m_LoopTime),
        AreBeziersRestricted: true,
        ...(meta ? { FadeInTime: number(meta.FadeInTime), FadeOutTime: number(meta.FadeOutTime) } : {}),
        CurveCount: out.length,
        TotalSegmentCount: segments,
        TotalPointCount: pointCount,
        UserDataCount: events.length,
        TotalUserDataSize: events.reduce((n, e) => n + String(e.data ?? '').length, 0),
      },
      Curves: out,
      UserData: events.map((e) => ({ Time: number(e.time), Value: e.data })),
    },
    jump,
  };
}

// ---------------------------------------------------------------------------------------------

function loadMotion(fw, json, asF32) {
  const bytes = new TextEncoder().encode(JSON.stringify(json));
  const buffer = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
  const motion = fw.CubismMotion.create(buffer, buffer.byteLength);
  const md = motion._motionData;
  if (asF32) for (const p of md.points) { p.time = f32(p.time); p.value = f32(p.value); }
  return md;
}

const VARIANTS = [
  ['exact', false, false],
  ['exact, 32-bit points', false, true],
  ['three decimals', true, false],
  ['three decimals, 32-bit points', true, true],
];

function newStats() {
  return { curves: 0, maxAbs: 0, maxAbsMidpoints: 0, curvesOver: THRESHOLDS.map(() => 0), valuesOver: THRESHOLDS.map(() => 0), worst: [] };
}

async function main() {
  const args = process.argv.slice(2);
  const reportAt = args.indexOf('--report');
  const report = reportAt >= 0 ? args.splice(reportAt, 2)[1] : null;
  const [frameworkDir, ...dirs] = args;
  if (!frameworkDir || dirs.length === 0) {
    console.error('usage: node compare.mjs <framework directory> <objects directory>... [--report <file>]');
    process.exit(2);
  }
  const fw = await loadFramework(frameworkDir);
  const summary = {
    clips: 0, curves: { streamed: 0, dense: 0, constant: 0 }, values: 0, largestJump: 0,
    skippedClips: [], loopingClips: 0, clipsWithFade: 0, events: {},
    variants: Object.fromEntries(VARIANTS.map(([name]) => [name, {}])),
  };
  for (const dir of dirs) {
    for (const file of objectDocuments(dir)) {
      for (const clip of clips(file)) {
        const bindings = clip.fields.m_ClipBindingConstant?.genericBindings ?? [];
        if (clip.fields.m_Compressed || bindings.some((b) => b.typeID !== MONO_BEHAVIOUR)) {
          summary.skippedClips.push({ bundle: clip.bundle, clip: clip.name, reason: 'not a Live2D parameter clip' });
          continue;
        }
        const curves = clipCurves(clip.fields);
        if (curves.length !== bindings.length) {
          summary.skippedClips.push({ bundle: clip.bundle, clip: clip.name, reason: 'curve and binding counts differ' });
          continue;
        }
        summary.clips++;
        const muscle = clip.fields.m_MuscleClip;
        if (muscle.m_LoopTime) summary.loopingClips++;
        if (clip.meta) summary.clipsWithFade++;
        for (const e of clip.fields.m_Events ?? []) summary.events[e.functionName] = (summary.events[e.functionName] ?? 0) + 1;
        const start = f32(number(muscle.m_StartTime));
        const length = f32(f32(number(muscle.m_StopTime)) - start);
        const { frames, mids } = sampleTimes(length);
        const local = [...frames, ...mids];
        const reference = curves.map((curve) => local.map((t) => evaluateUnity(curve, f32(start + t))));
        for (const curve of curves) summary.curves[curve.kind]++;
        summary.values += curves.length * local.length;
        for (const [name, round, asF32] of VARIANTS) {
          const written = motion3(clip, curves, round);
          if (name === 'exact') summary.largestJump = Math.max(summary.largestJump, written.jump);
          const md = loadMotion(fw, written.json, asF32);
          curves.forEach((curve, c) => {
            const stats = (summary.variants[name][curve.kind] ??= newStats());
            stats.curves++;
            let max = 0;
            let at = 0;
            let got = 0;
            const over = THRESHOLDS.map(() => 0);
            local.forEach((t, i) => {
              const v = fw.evaluateCurve(md, c, t, false, md.duration);
              const d = Math.abs(v - reference[c][i]);
              if (d > max) { max = d; at = i; got = v; }
              if (i >= frames.length) stats.maxAbsMidpoints = Math.max(stats.maxAbsMidpoints, d);
              THRESHOLDS.forEach((e, k) => { if (d > e) over[k]++; });
            });
            stats.maxAbs = Math.max(stats.maxAbs, max);
            over.forEach((n, k) => { stats.valuesOver[k] += n; if (n > 0) stats.curvesOver[k]++; });
            if (max > 0) {
              stats.worst.push({ bundle: clip.bundle, clip: clip.name, curve: c, binding: bindings[c].path, difference: max,
                time: local[at], unity: reference[c][at], cubism: got, midpoint: at >= frames.length });
              stats.worst.sort((x, y) => y.difference - x.difference);
              stats.worst.length = Math.min(stats.worst.length, 10);
            }
          });
        }
      }
    }
  }
  summary.thresholds = THRESHOLDS;
  if (report) fs.writeFileSync(report, `${JSON.stringify(summary, null, 1)}\n`);
  console.log(`${summary.clips} clips, ${summary.curves.streamed} streamed, ${summary.curves.dense} dense and ${summary.curves.constant} constant curves, ${summary.values} values`);
  console.log(`largest jump at a key ${summary.largestJump.toExponential(3)}, ${summary.loopingClips} looping clips, ${summary.clipsWithFade} clips with fade times, ${summary.skippedClips.length} clips skipped`);
  console.log(`events by function: ${JSON.stringify(summary.events)}`);
  for (const [name] of VARIANTS) {
    for (const [kind, stats] of Object.entries(summary.variants[name])) {
      const over = THRESHOLDS.map((e, k) => `>${e}: ${stats.curvesOver[k]}`).join(', ');
      console.log(`${name.padEnd(30)} ${kind.padEnd(9)} max ${stats.maxAbs.toExponential(3)} (midpoints ${stats.maxAbsMidpoints.toExponential(3)}); curves ${over}`);
    }
  }
  const exact = Object.values(summary.variants['exact, 32-bit points']);
  const worst = Math.max(0, ...exact.map((s) => s.maxAbs));
  if (worst > TOLERANCE) {
    console.log(`the exact motion3 read at 32-bit precision differs by ${worst}, more than ${TOLERANCE}`);
    process.exit(1);
  }
}

export { clipCurves, evaluateUnity, sampleTimes, motion3 };

if (import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) await main();
