import test from 'ava'
import { createServer } from 'node:net'
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { inflateSync } from 'node:zlib'
import { join } from 'node:path'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const { WadoServer, WadoStorageType } = process.env.NODE_DICOM_TEST_ADDON
  ? require(process.env.NODE_DICOM_TEST_ADDON)
  : require('../index')

function element(group, tag, vr, value) {
  const data = Buffer.isBuffer(value) ? value : Buffer.from(value, 'ascii')
  const padded = data.length % 2 ? Buffer.concat([data, Buffer.from([vr === 'UI' ? 0 : 32])]) : data
  const long = ['OB', 'OW', 'SQ', 'UN', 'UT'].includes(vr)
  const header = Buffer.alloc(long ? 12 : 8)
  header.writeUInt16LE(group, 0)
  header.writeUInt16LE(tag, 2)
  header.write(vr, 4)
  if (long) header.writeUInt32LE(padded.length, 8)
  else header.writeUInt16LE(padded.length, 6)
  return Buffer.concat([header, padded])
}

function us(value) {
  const buffer = Buffer.alloc(2)
  buffer.writeUInt16LE(value)
  return buffer
}

function item(data, tag = 0xe000) {
  const header = Buffer.alloc(8)
  header.writeUInt16LE(0xfffe, 0)
  header.writeUInt16LE(tag, 2)
  header.writeUInt32LE(data.length, 4)
  return Buffer.concat([header, data])
}

function rleFrame(pixels, bits = 16) {
  const header = Buffer.alloc(64)
  if (bits === 8) {
    header.writeUInt32LE(1, 0)
    header.writeUInt32LE(64, 4)
    return Buffer.concat([header, Buffer.from([3, ...pixels, 0])])
  }
  header.writeUInt32LE(2, 0)
  header.writeUInt32LE(64, 4)
  header.writeUInt32LE(70, 8)
  const high = Buffer.from([3, ...pixels.map((value) => value >> 8), 0])
  const low = Buffer.from([3, ...pixels.map((value) => value & 255), 0])
  return Buffer.concat([header, high, low])
}

function jpegLosslessFrame(bits) {
  return Buffer.from([
    0xff,
    0xd8,
    0xff,
    0xc3,
    0,
    11,
    bits,
    0,
    2,
    0,
    2,
    1,
    1,
    0x11,
    0,
    0xff,
    0xc4,
    0,
    20,
    0,
    1,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0xff,
    0xda,
    0,
    8,
    1,
    1,
    0,
    1,
    0,
    0,
    0x0f,
    0xff,
    0xd9,
    0,
  ])
}

function dicom(encoding, frames, color = false, extraTags = [], mono1 = false, signed = false) {
  const compressed = !encoding.startsWith('raw')
  const bits = encoding.endsWith('8') ? 8 : 16
  const transferSyntax = encoding.startsWith('rle')
    ? '1.2.840.10008.1.2.5'
    : encoding.startsWith('jpeg')
      ? '1.2.840.10008.1.2.4.70'
      : '1.2.840.10008.1.2.1'
  const sopClass = '1.2.840.10008.5.1.4.1.1.2'
  const meta = Buffer.concat([
    element(2, 1, 'OB', Buffer.from([0, 1])),
    element(2, 2, 'UI', sopClass),
    element(2, 3, 'UI', '1.2.3.4.5'),
    element(2, 0x10, 'UI', transferSyntax),
    element(2, 0x12, 'UI', '1.2.826.0.1.3680043.10.999'),
  ])
  const metaLength = Buffer.alloc(4)
  metaLength.writeUInt32LE(meta.length)
  const tags = [
    element(8, 0x16, 'UI', sopClass),
    element(8, 0x18, 'UI', '1.2.3.4.5'),
    element(0x20, 0x0d, 'UI', '1.2.3'),
    element(0x20, 0x0e, 'UI', '1.2.3.4'),
    element(0x28, 2, 'US', us(color ? 3 : 1)),
    element(0x28, 4, 'CS', color ? 'RGB' : mono1 ? 'MONOCHROME1' : 'MONOCHROME2'),
    element(0x28, 8, 'IS', String(frames.length)),
    element(0x28, 0x10, 'US', us(2)),
    element(0x28, 0x11, 'US', us(2)),
    element(0x28, 0x100, 'US', us(bits)),
    element(0x28, 0x101, 'US', us(bits)),
    element(0x28, 0x102, 'US', us(bits - 1)),
    element(0x28, 0x103, 'US', us(signed ? 1 : 0)),
    ...extraTags,
  ]
  let pixelData
  if (compressed) {
    const header = Buffer.alloc(12)
    header.writeUInt16LE(0x7fe0, 0)
    header.writeUInt16LE(0x10, 2)
    header.write('OB', 4)
    header.writeUInt32LE(0xffffffff, 8)
    const fragments = frames.map((frame) =>
      encoding.startsWith('jpeg') ? jpegLosslessFrame(bits) : rleFrame(frame, bits)
    )
    let offset = 0
    const offsets = Buffer.alloc(fragments.length * 4)
    for (const [index, fragment] of fragments.entries()) {
      offsets.writeUInt32LE(offset, index * 4)
      offset += fragment.length + 8
    }
    pixelData = Buffer.concat([
      header,
      item(offsets),
      ...fragments.map((frame) => item(frame)),
      item(Buffer.alloc(0), 0xe0dd),
    ])
  } else {
    pixelData = element(
      0x7fe0,
      0x10,
      bits === 8 ? 'OB' : 'OW',
      bits === 8 ? Buffer.from(frames.flat()) : Buffer.concat(frames.flat().map((value) => us(value & 0xffff)))
    )
  }
  return Buffer.concat([
    Buffer.alloc(128),
    Buffer.from('DICM'),
    element(2, 0, 'UL', metaLength),
    meta,
    ...tags,
    pixelData,
  ])
}

async function freePort() {
  const server = createServer()
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', resolve)
  })
  const port = server.address().port
  await new Promise((resolve, reject) => server.close((error) => (error ? reject(error) : resolve())))
  return port
}

async function fetchWhenReady(url, options) {
  for (let attempt = 0; attempt < 100; attempt++) {
    try {
      return await fetch(url, options)
    } catch (error) {
      if (attempt === 99) throw error
      await new Promise((resolve) => setTimeout(resolve, 20))
    }
  }
}

// Decode the RGB8 PNG returned by the renderer without adding a test dependency.
function pngImage(buffer) {
  const width = buffer.readUInt32BE(16)
  const height = buffer.readUInt32BE(20)
  if (buffer[24] !== 8 || buffer[25] !== 2) throw new Error('Expected RGB8 PNG')
  const chunks = []
  for (let offset = 8; offset < buffer.length; ) {
    const length = buffer.readUInt32BE(offset)
    if (buffer.toString('ascii', offset + 4, offset + 8) === 'IDAT') {
      chunks.push(buffer.subarray(offset + 8, offset + 8 + length))
    }
    offset += length + 12
  }
  const filtered = inflateSync(Buffer.concat(chunks))
  const stride = width * 3
  const pixels = Buffer.alloc(stride * height)
  const paeth = (a, b, c) => {
    const p = a + b - c
    const [pa, pb, pc] = [Math.abs(p - a), Math.abs(p - b), Math.abs(p - c)]
    return pa <= pb && pa <= pc ? a : pb <= pc ? b : c
  }
  for (let y = 0; y < height; y++) {
    const filter = filtered[y * (stride + 1)]
    for (let x = 0; x < stride; x++) {
      const i = y * stride + x
      const a = x >= 3 ? pixels[i - 3] : 0
      const b = y ? pixels[i - stride] : 0
      const c = y && x >= 3 ? pixels[i - stride - 3] : 0
      const prediction = [0, a, b, Math.floor((a + b) / 2), paeth(a, b, c)][filter]
      if (prediction === undefined) throw new Error('Invalid PNG filter')
      pixels[i] = filtered[y * (stride + 1) + x + 1] + prediction
    }
  }
  return { width, height, gray: [...pixels].filter((_, i) => i % 3 === 0) }
}

function sequence(group, tag, items) {
  return element(group, tag, 'SQ', Buffer.concat(items.map((data) => item(Buffer.concat(data)))))
}

function rescale(slope, intercept) {
  return sequence(0x28, 0x9145, [
    [element(0x28, 0x1052, 'DS', String(intercept)), element(0x28, 0x1053, 'DS', String(slope))],
  ])
}

function windowLevel(center, width) {
  return sequence(0x28, 0x9132, [
    [element(0x28, 0x1050, 'DS', String(center)), element(0x28, 0x1051, 'DS', String(width))],
  ])
}

test('WADO selected-frame rendered previews decode first/middle/last with Enhanced metadata and validate frame numbers', async (t) => {
  const root = await mkdtemp(join(process.cwd(), '.wado-rendered-'))
  const dir = join(root, '1.2.3', '1.2.3.4')
  await mkdir(dir, { recursive: true })
  const frames = [
    [0, 10, 20, 30],
    [10, 20, 30, 40],
    [20, 30, 40, 50],
  ]
  const enhanced = [
    element(0x28, 0x1052, 'DS', '500'),
    element(0x28, 0x1053, 'DS', '9'),
    element(0x28, 0x1050, 'DS', '800'),
    element(0x28, 0x1051, 'DS', '1000'),
    sequence(0x5200, 0x9229, [[rescale(2, 10), windowLevel(128, 256)]]),
    sequence(0x5200, 0x9230, [[], [rescale(3, 20), windowLevel(256, 512)], [rescale(4, 30)]]),
  ]
  const expected = [
    [10, 30, 50, 70],
    [25, 40, 55, 70],
    [110, 150, 190, 230],
  ]
  const files = [
    ['10', 'raw', enhanced, false],
    ['11', 'rle', enhanced, false],
    ['12', 'raw8', enhanced, false],
    ['13', 'rle8', enhanced, false],
    ['14', 'rle', enhanced, true],
    ['15', 'jpeg8', [], false],
    ['16', 'jpeg16', [], false],
  ]
  for (const [id, encoding, metadata, mono1] of files) {
    await writeFile(join(dir, `1.2.3.4.${id}.dcm`), dicom(encoding, frames, false, metadata, mono1))
  }
  await writeFile(join(dir, '1.2.3.4.17.dcm'), dicom('jpeg8', frames, true))
  await writeFile(join(dir, '1.2.3.4.18.dcm'), dicom('raw', [[-100, 0, 100, 200]], false, [], false, true))
  await writeFile(join(dir, '1.2.3.4.19.dcm'), dicom('raw', frames))
  for (const [id, encoding] of [
    ['20', 'rle8'],
    ['21', 'rle'],
    ['22', 'jpeg8'],
    ['23', 'jpeg16'],
  ]) {
    await writeFile(join(dir, `1.2.3.4.${id}.dcm`), dicom(encoding, [[0, 64, 128, 255]]))
  }
  const port = await freePort()
  const disabledPort = await freePort()
  const options = {
    storageType: WadoStorageType.Filesystem,
    basePath: root,
    bindAddress: '127.0.0.1',
  }
  const server = new WadoServer(port, {
    ...options,
    enableRendered: true,
    enableThumbnail: true,
    enableFrames: false,
  })
  const disabled = new WadoServer(disabledPort, {
    ...options,
    enableRendered: false,
  })
  server.start()
  disabled.start()
  try {
    const path = '/studies/1.2.3/series/1.2.3.4/instances'
    const base = `http://127.0.0.1:${port}${path}`
    const png = { headers: { Accept: 'image/png' } }
    for (const [id] of files.slice(0, 5)) {
      for (let frame = 1; frame <= 3; frame++) {
        const response = await fetchWhenReady(`${base}/1.2.3.4.${id}/frames/${frame}/rendered`, png)
        t.is(response.status, 200, await response.clone().text())
        t.is(response.headers.get('content-type'), 'image/png')
        const image = pngImage(Buffer.from(await response.arrayBuffer()))
        t.deepEqual(
          image,
          {
            width: 2,
            height: 2,
            gray: expected[frame - 1].map((p) => (id === '14' ? 255 - p : p)),
          },
          `instance ${id}, frame ${frame}`
        )
      }
    }
    for (const [id, expectedValue] of [
      ['15', 128],
      ['16', 128],
    ]) {
      for (const frame of [1, 2, 3]) {
        const response = await fetch(
          `${base}/1.2.3.4.${id}/frames/${frame}/rendered?window=${id === '15' ? '128,256' : '32768,65536'}`,
          png
        )
        t.is(response.status, 200, await response.clone().text())
        t.deepEqual(pngImage(Buffer.from(await response.arrayBuffer())).gray, Array(4).fill(expectedValue))
      }
    }
    const overridden = await fetch(`${base}/1.2.3.4.11/frames/2/rendered?window=128,256`, png)
    t.deepEqual(pngImage(Buffer.from(await overridden.arrayBuffer())).gray, [50, 80, 110, 140])
    const signed = await fetch(`${base}/1.2.3.4.18/frames/1/rendered?window=0.5,201`, png)
    t.deepEqual(pngImage(Buffer.from(await signed.arrayBuffer())).gray, [0, 128, 255, 255])
    const resized = await fetch(`${base}/1.2.3.4.11/frames/3/rendered?viewport=8,6`, png)
    const resizedImage = pngImage(Buffer.from(await resized.arrayBuffer()))
    t.is(resizedImage.width, 8)
    t.is(resizedImage.height, 6)
    const jpeg = await fetch(`${base}/1.2.3.4.11/frames/2/rendered?viewport=7,5&quality=75`)
    t.is(jpeg.status, 200)
    t.is(jpeg.headers.get('content-type'), 'image/jpeg')
    const jpegBytes = Buffer.from(await jpeg.arrayBuffer())
    t.deepEqual([...jpegBytes.subarray(0, 2)], [255, 216])
    const sof = jpegBytes.indexOf(Buffer.from([255, 192]))
    t.true(sof >= 0)
    t.is(jpegBytes.readUInt16BE(sof + 5), 5)
    t.is(jpegBytes.readUInt16BE(sof + 7), 7)
    for (const [id, center, width, expectedPixels] of [
      ['20', 128, 256, [0, 64, 128, 255]],
      ['21', 128, 256, [0, 64, 128, 255]],
      ['22', 128, 256, [128, 128, 128, 128]],
      ['23', 32768, 65536, [128, 128, 128, 128]],
    ]) {
      const response = await fetch(`${base}/1.2.3.4.${id}/rendered?window=${center},${width}`, png)
      t.is(response.status, 200, await response.clone().text())
      t.deepEqual(pngImage(Buffer.from(await response.arrayBuffer())).gray, expectedPixels)
      for (const suffix of ['rendered', 'frames/1/rendered', 'thumbnail']) {
        const preview = await fetch(`${base}/1.2.3.4.${id}/${suffix}?viewport=160,120&quality=80`, {
          headers: { Accept: 'image/jpeg' },
        })
        t.is(preview.status, 200, `${id}/${suffix}: ${await preview.clone().text()}`)
        t.is(preview.headers.get('content-type'), 'image/jpeg')
        const bytes = Buffer.from(await preview.arrayBuffer())
        t.deepEqual([...bytes.subarray(0, 2)], [255, 216])
        t.deepEqual([...bytes.subarray(-2)], [255, 217])
        const startOfFrame = bytes.indexOf(Buffer.from([255, 192]))
        t.true(startOfFrame >= 0)
        t.is(bytes.readUInt16BE(startOfFrame + 5), 120)
        t.is(bytes.readUInt16BE(startOfFrame + 7), 160)
      }
    }
    for (const frame of ['0', '-1', '+1', '1.5', 'abc', '1,2', '4', '4294967296']) {
      const response = await fetch(`${base}/1.2.3.4.11/frames/${frame}/rendered`)
      t.is(response.status, 400, `${frame}: ${await response.clone().text()}`)
      t.regex((await response.json()).error, /frame/i)
    }
    const color = await fetch(`${base}/1.2.3.4.17/frames/1/rendered`)
    t.is(color.status, 500)
    t.regex((await color.json()).error, /Unsupported photometric/)
    for (const suffix of ['rendered', 'frames/1/rendered', 'frames/0/rendered']) {
      const response = await fetchWhenReady(`http://127.0.0.1:${disabledPort}${path}/1.2.3.4.11/${suffix}`)
      t.is(response.status, 404)
    }
    const legacy = await fetch(`${base}/1.2.3.4.19/rendered`, png)
    const first = await fetch(`${base}/1.2.3.4.19/frames/1/rendered`, png)
    t.is(legacy.status, 200)
    // Legacy instance rendering retains its all-frame auto range; selection uses only that frame.
    t.deepEqual(pngImage(Buffer.from(await legacy.arrayBuffer())).gray, [0, 51, 102, 153])
    t.deepEqual(pngImage(Buffer.from(await first.arrayBuffer())).gray, [0, 85, 170, 255])
  } finally {
    server.stop()
    disabled.stop()
    await rm(root, { recursive: true, force: true })
  }
})

test('WADO instance previews render compressed DICOM with production JPEG dimensions', async (t) => {
  const root = await mkdtemp(join(process.cwd(), '.wado-instance-preview-'))
  const dir = join(root, '1.2.3', '1.2.3.4')
  await mkdir(dir, { recursive: true })
  const encodings = ['raw8', 'raw', 'rle8', 'rle', 'jpeg8', 'jpeg16']
  for (const [index, encoding] of encodings.entries()) {
    await writeFile(join(dir, `1.2.3.4.${index}.dcm`), dicom(encoding, [[0, 64, 128, 255]]))
  }
  const port = await freePort()
  const server = new WadoServer(port, {
    storageType: WadoStorageType.Filesystem,
    basePath: root,
    bindAddress: '127.0.0.1',
    enableRendered: true,
  })
  server.start()
  try {
    for (const [index, encoding] of encodings.entries()) {
      const response = await fetchWhenReady(
        `http://127.0.0.1:${port}/studies/1.2.3/series/1.2.3.4/instances/1.2.3.4.${index}/rendered?viewport=160,120&quality=80`,
        { headers: { Accept: 'image/jpeg' } }
      )
      t.is(response.status, 200, response.ok ? encoding : `${encoding}: ${await response.clone().text()}`)
      t.is(response.headers.get('content-type'), 'image/jpeg')
      const bytes = Buffer.from(await response.arrayBuffer())
      t.deepEqual([...bytes.subarray(0, 2)], [255, 216])
      t.deepEqual([...bytes.subarray(-2)], [255, 217])
      const startOfFrame = bytes.indexOf(Buffer.from([255, 192]))
      t.true(startOfFrame >= 0)
      t.is(bytes.readUInt16BE(startOfFrame + 5), 120)
      t.is(bytes.readUInt16BE(startOfFrame + 7), 160)
    }
  } finally {
    server.stop()
    await rm(root, { recursive: true, force: true })
  }
})

test('WADO frame retrieval decodes RLE and JPEG lossless frames and preserves uncompressed frames', async (t) => {
  const root = await mkdtemp(join(process.cwd(), '.wado-frames-'))
  const dir = join(root, '1.2.3', '1.2.3.4')
  await mkdir(dir, { recursive: true })
  const frames = [
    [0x0102, 0x1234, 0xabcd, 0xffff],
    [0, 1, 0x8000, 0xff00],
  ]
  await writeFile(join(dir, '1.2.3.4.5.dcm'), dicom('rle', frames))
  await writeFile(join(dir, '1.2.3.4.6.dcm'), dicom('raw', frames))
  await writeFile(join(dir, '1.2.3.4.7.dcm'), dicom('jpeg8', [[128, 128, 128, 128]]))
  await writeFile(join(dir, '1.2.3.4.8.dcm'), dicom('jpeg16', [[32768, 32768, 32768, 32768]]))
  await writeFile(join(dir, '1.2.3.4.9.dcm'), dicom('jpeg8', [[128, 128, 128, 128]], true))
  const port = await freePort()
  const server = new WadoServer(port, {
    storageType: WadoStorageType.Filesystem,
    basePath: root,
    bindAddress: '127.0.0.1',
  })
  server.start()
  try {
    const base = `http://127.0.0.1:${port}/studies/1.2.3/series/1.2.3.4/instances`
    for (const instance of ['1.2.3.4.5', '1.2.3.4.6']) {
      for (const [index, expected] of frames.entries()) {
        const response = await fetchWhenReady(`${base}/${instance}/frames/${index + 1}`, {
          headers: {
            Accept: 'multipart/related; type=application/octet-stream; transfer-syntax=*',
          },
        })
        t.is(response.status, 200, await response.clone().text())
        t.regex(response.headers.get('content-type'), /transfer-syntax=1\.2\.840\.10008\.1\.2\.1/)
        const body = Buffer.from(await response.arrayBuffer())
        const offset = body.indexOf('\r\n\r\n') + 4
        t.true(offset >= 4)
        t.regex(
          body.subarray(0, offset).toString(),
          /Content-Type: application\/octet-stream; transfer-syntax=1\.2\.840\.10008\.1\.2\.1/
        )
        t.deepEqual(body.subarray(offset, offset + 8), Buffer.concat(expected.map(us)))
      }
    }
    for (const [instance, expected] of [
      ['1.2.3.4.7', Buffer.from([128, 128, 128, 128])],
      ['1.2.3.4.8', Buffer.concat([32768, 32768, 32768, 32768].map(us))],
    ]) {
      const jpeg = await fetch(`${base}/${instance}/frames/1`, {
        headers: {
          Accept: 'multipart/related; type="application/octet-stream"; transfer-syntax=*',
        },
      })
      t.is(jpeg.status, 200, await jpeg.clone().text())
      const jpegBody = Buffer.from(await jpeg.arrayBuffer())
      const jpegOffset = jpegBody.indexOf('\r\n\r\n') + 4
      t.deepEqual(jpegBody.subarray(jpegOffset, jpegOffset + expected.length), expected)
    }
    const multi = await fetch(`${base}/1.2.3.4.5/frames/1,2`)
    t.is(multi.status, 200)
    const boundary = /boundary="([^"]+)"/.exec(multi.headers.get('content-type'))[1]
    const body = Buffer.from(await multi.arrayBuffer())
    const parts = body.toString('latin1').split(`--${boundary}`).slice(1, -1)
    t.is(parts.length, 2)
    for (const [index, part] of parts.entries()) {
      const bytes = Buffer.from(part.slice(part.indexOf('\r\n\r\n') + 4, -2), 'latin1')
      t.deepEqual(bytes, Buffer.concat(frames[index].map(us)))
    }
    const unsupported = await fetch(`${base}/1.2.3.4.5/frames/1`, {
      headers: { Accept: 'image/jpeg' },
    })
    t.is(unsupported.status, 406)
    const wrongTransferSyntax = await fetch(`${base}/1.2.3.4.5/frames/1`, {
      headers: {
        Accept: 'multipart/related; type=application/octet-stream; transfer-syntax=1.2.840.10008.1.2.4.70',
      },
    })
    t.is(wrongTransferSyntax.status, 406)
    const noAcceptableFormat = await fetch(`${base}/1.2.3.4.5/frames/1`, {
      headers: {
        Accept: 'multipart/related; type=application/octet-stream; q=0',
      },
    })
    t.is(noAcceptableFormat.status, 406)
    const color = await fetch(`${base}/1.2.3.4.9/frames/1`)
    t.is(color.status, 500)
    t.regex((await color.json()).error, /Compressed color/)
    const outOfRange = await fetch(`${base}/1.2.3.4.5/frames/3`)
    t.is(outOfRange.status, 500)
    t.regex((await outOfRange.json()).error, /out of range/)
  } finally {
    server.stop()
    await rm(root, { recursive: true, force: true })
  }
})

const jpegSamples = process.env.NODE_DICOM_JPEG_SAMPLES
;(jpegSamples ? test : test.skip)('WADO renders real JPEG lossless 8/16-bit samples and rejects color', async (t) => {
  const root = await mkdtemp(join(process.cwd(), '.wado-jpeg-'))
  const dir = join(root, '1.2.3', '1.2.3.4')
  await mkdir(dir, { recursive: true })
  for (const [id, name] of [
    ['30', 'jpeg_lossless_sel1.dcm'],
    ['31', 'jpeg_lossless_sel1-8bit.dcm'],
    ['32', 'jpeg_lossless_sel1-rgb.dcm'],
  ]) {
    await writeFile(join(dir, `1.2.3.4.${id}.dcm`), await readFile(join(jpegSamples, name)))
  }
  const port = await freePort()
  const server = new WadoServer(port, {
    storageType: WadoStorageType.Filesystem,
    basePath: root,
    bindAddress: '127.0.0.1',
    enableRendered: true,
  })
  server.start()
  try {
    const base = `http://127.0.0.1:${port}/studies/1.2.3/series/1.2.3.4/instances`
    for (const id of ['30', '31']) {
      for (const suffix of ['rendered', 'frames/1/rendered']) {
        const png = await fetchWhenReady(`${base}/1.2.3.4.${id}/${suffix}?viewport=160,120`, {
          headers: { Accept: 'image/png' },
        })
        t.is(png.status, 200, await png.clone().text())
        const image = pngImage(Buffer.from(await png.arrayBuffer()))
        t.is(image.width, 160)
        t.is(image.height, 120)
        t.true(new Set(image.gray).size > 10)
        const jpeg = await fetch(`${base}/1.2.3.4.${id}/${suffix}?viewport=160,120&quality=80`, {
          headers: { Accept: 'image/jpeg' },
        })
        t.is(jpeg.status, 200, await jpeg.clone().text())
        t.is(jpeg.headers.get('content-type'), 'image/jpeg')
        t.true((await jpeg.arrayBuffer()).byteLength > 100)
      }
    }
    for (const suffix of ['rendered', 'frames/1/rendered']) {
      const color = await fetch(`${base}/1.2.3.4.32/${suffix}`)
      t.is(color.status, 500)
      t.regex((await color.json()).error, /Unsupported photometric/)
    }
  } finally {
    server.stop()
    await rm(root, { recursive: true, force: true })
  }
})
