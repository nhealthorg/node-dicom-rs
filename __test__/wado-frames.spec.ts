import test from 'ava'
import { createServer } from 'node:net'
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
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

function rleFrame(pixels) {
  const header = Buffer.alloc(64)
  header.writeUInt32LE(2, 0)
  header.writeUInt32LE(64, 4)
  header.writeUInt32LE(70, 8)
  const high = Buffer.from([3, ...pixels.map(value => value >> 8), 0])
  const low = Buffer.from([3, ...pixels.map(value => value & 255), 0])
  return Buffer.concat([header, high, low])
}

function jpegLosslessFrame(bits) {
  return Buffer.from([
    0xff, 0xd8,
    0xff, 0xc3, 0, 11, bits, 0, 2, 0, 2, 1, 1, 0x11, 0,
    0xff, 0xc4, 0, 20, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0xff, 0xda, 0, 8, 1, 1, 0, 1, 0, 0,
    0x0f, 0xff, 0xd9, 0,
  ])
}

function dicom(encoding, frames, color = false) {
  const compressed = encoding !== 'raw'
  const bits = encoding === 'jpeg8' ? 8 : 16
  const transferSyntax = encoding === 'rle' ? '1.2.840.10008.1.2.5'
    : encoding.startsWith('jpeg') ? '1.2.840.10008.1.2.4.70' : '1.2.840.10008.1.2.1'
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
    element(0x28, 4, 'CS', color ? 'RGB' : 'MONOCHROME2'),
    element(0x28, 8, 'IS', String(frames.length)),
    element(0x28, 0x10, 'US', us(2)),
    element(0x28, 0x11, 'US', us(2)),
    element(0x28, 0x100, 'US', us(bits)),
    element(0x28, 0x101, 'US', us(bits)),
    element(0x28, 0x102, 'US', us(bits - 1)),
    element(0x28, 0x103, 'US', us(0)),
  ]
  let pixelData
  if (compressed) {
    const header = Buffer.alloc(12)
    header.writeUInt16LE(0x7fe0, 0)
    header.writeUInt16LE(0x10, 2)
    header.write('OB', 4)
    header.writeUInt32LE(0xffffffff, 8)
    const fragments = frames.map(frame => encoding.startsWith('jpeg') ? jpegLosslessFrame(bits) : rleFrame(frame))
    let offset = 0
    const offsets = Buffer.alloc(fragments.length * 4)
    for (const [index, fragment] of fragments.entries()) {
      offsets.writeUInt32LE(offset, index * 4)
      offset += fragment.length + 8
    }
    pixelData = Buffer.concat([header, item(offsets), ...fragments.map(frame => item(frame)), item(Buffer.alloc(0), 0xe0dd)])
  } else {
    pixelData = element(0x7fe0, 0x10, 'OW', Buffer.concat(frames.flat().map(us)))
  }
  return Buffer.concat([Buffer.alloc(128), Buffer.from('DICM'), element(2, 0, 'UL', metaLength), meta, ...tags, pixelData])
}

async function freePort() {
  const server = createServer()
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', resolve)
  })
  const port = server.address().port
  await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()))
  return port
}

async function fetchWhenReady(url, options) {
  for (let attempt = 0; attempt < 100; attempt++) {
    try { return await fetch(url, options) } catch (error) {
      if (attempt === 99) throw error
      await new Promise(resolve => setTimeout(resolve, 20))
    }
  }
}

test('WADO frame retrieval decodes RLE and JPEG lossless frames and preserves uncompressed frames', async t => {
  const root = await mkdtemp(join(tmpdir(), 'wado-frames-'))
  const dir = join(root, '1.2.3', '1.2.3.4')
  await mkdir(dir, { recursive: true })
  const frames = [[0x0102, 0x1234, 0xabcd, 0xffff], [0, 1, 0x8000, 0xff00]]
  await writeFile(join(dir, '1.2.3.4.5.dcm'), dicom('rle', frames))
  await writeFile(join(dir, '1.2.3.4.6.dcm'), dicom('raw', frames))
  await writeFile(join(dir, '1.2.3.4.7.dcm'), dicom('jpeg8', [[128, 128, 128, 128]]))
  await writeFile(join(dir, '1.2.3.4.8.dcm'), dicom('jpeg16', [[32768, 32768, 32768, 32768]]))
  await writeFile(join(dir, '1.2.3.4.9.dcm'), dicom('jpeg8', [[128, 128, 128, 128]], true))
  const port = await freePort()
  const server = new WadoServer(port, { storageType: WadoStorageType.Filesystem, basePath: root, bindAddress: '127.0.0.1' })
  server.start()
  try {
    const base = `http://127.0.0.1:${port}/studies/1.2.3/series/1.2.3.4/instances`
    for (const instance of ['1.2.3.4.5', '1.2.3.4.6']) {
      for (const [index, expected] of frames.entries()) {
        const response = await fetchWhenReady(`${base}/${instance}/frames/${index + 1}`, {
          headers: { Accept: 'multipart/related; type=application/octet-stream; transfer-syntax=*' },
        })
        t.is(response.status, 200, await response.clone().text())
        t.regex(response.headers.get('content-type'), /transfer-syntax=1\.2\.840\.10008\.1\.2\.1/)
        const body = Buffer.from(await response.arrayBuffer())
        const offset = body.indexOf('\r\n\r\n') + 4
        t.true(offset >= 4)
        t.regex(body.subarray(0, offset).toString(), /Content-Type: application\/octet-stream; transfer-syntax=1\.2\.840\.10008\.1\.2\.1/)
        t.deepEqual(body.subarray(offset, offset + 8), Buffer.concat(expected.map(us)))
      }
    }
    for (const [instance, expected] of [
      ['1.2.3.4.7', Buffer.from([128, 128, 128, 128])],
      ['1.2.3.4.8', Buffer.concat([32768, 32768, 32768, 32768].map(us))],
    ]) {
      const jpeg = await fetch(`${base}/${instance}/frames/1`, {
        headers: { Accept: 'multipart/related; type="application/octet-stream"; transfer-syntax=*' },
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
    const unsupported = await fetch(`${base}/1.2.3.4.5/frames/1`, { headers: { Accept: 'image/jpeg' } })
    t.is(unsupported.status, 406)
    const wrongTransferSyntax = await fetch(`${base}/1.2.3.4.5/frames/1`, {
      headers: { Accept: 'multipart/related; type=application/octet-stream; transfer-syntax=1.2.840.10008.1.2.4.70' },
    })
    t.is(wrongTransferSyntax.status, 406)
    const noAcceptableFormat = await fetch(`${base}/1.2.3.4.5/frames/1`, {
      headers: { Accept: 'multipart/related; type=application/octet-stream; q=0' },
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
