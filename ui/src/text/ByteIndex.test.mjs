// ByteIndex 单测。运行:`node ui/src/text/ByteIndex.test.mjs`
// 阶段二暂不引入 Vitest,assert-based 测试 + npm test 脚本足以。
import { strict as assert } from "node:assert";
import { ByteIndex } from "./ByteIndex.js";

function test(name, fn) {
  try {
    fn();
    console.log(`  ✓ ${name}`);
  } catch (e) {
    console.error(`  ✗ ${name}`);
    console.error(`    ${e.message}`);
    process.exitCode = 1;
  }
}

console.log("ByteIndex:");

test("ASCII: charToByte and byteToChar are identity", () => {
  const ix = new ByteIndex("hello");
  assert.equal(ix.totalBytes, 5);
  for (let i = 0; i <= 5; i++) {
    assert.equal(ix.charToByte(i), i);
    assert.equal(ix.byteToChar(i), i);
  }
});

test("CJK BMP: each char = 3 UTF-8 bytes", () => {
  // "你好" = 2 chars, 6 bytes
  const ix = new ByteIndex("你好");
  assert.equal(ix.totalBytes, 6);
  assert.equal(ix.charToByte(0), 0);
  assert.equal(ix.charToByte(1), 3);
  assert.equal(ix.charToByte(2), 6);
  assert.equal(ix.byteToChar(0), 0);
  assert.equal(ix.byteToChar(2), 0, "字节 2 应落在「你」内");
  assert.equal(ix.byteToChar(3), 1, "字节 3 应是「好」起点");
  assert.equal(ix.byteToChar(5), 1, "字节 5 应落在「好」内");
  assert.equal(ix.byteToChar(6), 2);
});

test("mixed ASCII + CJK", () => {
  // "hi你好" = 4 UTF-16 units, 1+1+3+3 = 8 bytes
  const ix = new ByteIndex("hi你好");
  assert.equal(ix.totalBytes, 8);
  assert.equal(ix.charToByte(0), 0);
  assert.equal(ix.charToByte(1), 1);
  assert.equal(ix.charToByte(2), 2);
  assert.equal(ix.charToByte(3), 5);
  assert.equal(ix.charToByte(4), 8);
  assert.equal(ix.byteToChar(0), 0);
  assert.equal(ix.byteToChar(1), 1);
  assert.equal(ix.byteToChar(2), 2);
  assert.equal(ix.byteToChar(4), 2, "字节 4 在「你」中");
  assert.equal(ix.byteToChar(5), 3);
});

test("surrogate pair (emoji): 2 UTF-16 units = 4 UTF-8 bytes", () => {
  // "🎉" = 1 codepoint = 2 UTF-16 units = 4 UTF-8 bytes
  const ix = new ByteIndex("a🎉b");
  // UTF-16: a(1) + 🎉(2) + b(1) = 4 units
  // UTF-8:  a(1) + 🎉(4) + b(1) = 6 bytes
  assert.equal(ix.text.length, 4);
  assert.equal(ix.totalBytes, 6);
  assert.equal(ix.charToByte(0), 0); // 'a' starts at byte 0
  assert.equal(ix.charToByte(1), 1); // high surrogate starts at byte 1
  assert.equal(ix.charToByte(2), 5); // low surrogate position → byte 5(对外位置 = 紧随 4 字节之后)
  assert.equal(ix.charToByte(3), 5); // 'b' starts at byte 5
  assert.equal(ix.charToByte(4), 6);
  // 反向:byte 1..4 都属于 emoji 的高代理位
  for (let b = 1; b <= 4; b++) {
    assert.equal(ix.byteToChar(b), 1, `byte ${b} 应落在 emoji 高代理`);
  }
  assert.equal(ix.byteToChar(5), 3, "byte 5 应是 'b'");
});

test("spanToCharRange: end-exclusive", () => {
  const ix = new ByteIndex("hi你好");
  // span = [2, 5) = 「你」
  const r = ix.spanToCharRange({ start: 2, end: 5 });
  assert.equal(r.start, 2);
  assert.equal(r.end, 3);
  assert.equal(ix.text.slice(r.start, r.end), "你");
});

test("out-of-range clamps gracefully", () => {
  const ix = new ByteIndex("abc");
  assert.equal(ix.charToByte(-1), 0);
  assert.equal(ix.charToByte(99), 3);
  assert.equal(ix.byteToChar(-5), 0);
  assert.equal(ix.byteToChar(99), 3);
});

if (process.exitCode === 1) {
  console.error("\nByteIndex 测试失败");
} else {
  console.log("\nAll ByteIndex tests passed.");
}
