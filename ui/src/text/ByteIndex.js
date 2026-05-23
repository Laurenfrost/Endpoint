// 富标注契约规定 span 的偏移单位是 **UTF-8 字节**(`crates/core/src/domain.rs` 模块文档
// 第 1 条);而 JS 字符串是 UTF-16 编码。每次显示一个高亮区间都需要把 byte → UTF-16 index
// 换算一次。整本两百万字的小说会有上万次查询,实时按字符 walk 必然炸,本类一次性建好索引
// 后所有查询都是 O(log n)。
//
// 数据结构:`byteOffsets` 是 Uint32Array,长度 = `text.length + 1`(UTF-16 code unit 数 + 1)。
//   byteOffsets[i] = UTF-16 第 i 个 code unit 在 UTF-8 中的起始字节偏移。
//   byteOffsets[n] = 文本总字节数。
//
// 内存:2M 字符 → 8MB。可接受。如未来需要省内存,可改用稀疏 checkpoint + 线性扫描。

export class ByteIndex {
  constructor(text) {
    const n = text.length;
    this.text = text;
    this.byteOffsets = new Uint32Array(n + 1);
    let b = 0;
    for (let i = 0; i < n; i++) {
      this.byteOffsets[i] = b;
      const code = text.charCodeAt(i);
      if (code < 0x80) {
        b += 1;
      } else if (code < 0x800) {
        b += 2;
      } else if (code >= 0xd800 && code <= 0xdbff && i + 1 < n) {
        // 高代理 + 低代理 = 一个 BMP 之外的码点,UTF-8 占 4 字节。
        b += 4;
        i += 1;
        // 低代理位也是这一对的一部分,记录其 byte 起点 = 紧跟在 4 字节之后
        // (这样 byteToChar 二分时不会错误地落到低代理)。
        this.byteOffsets[i] = b;
      } else {
        // BMP 0x0800-0xFFFF 占 3 字节(包含未配对的低代理,容错)。
        b += 3;
      }
    }
    this.byteOffsets[n] = b;
    this.totalBytes = b;
  }

  /// UTF-16 code unit i 在 UTF-8 中的起始字节偏移。
  charToByte(charIdx) {
    if (charIdx <= 0) return 0;
    if (charIdx >= this.byteOffsets.length) return this.totalBytes;
    return this.byteOffsets[charIdx];
  }

  /// UTF-8 字节偏移对应的 UTF-16 code unit 位置。
  /// 返回最大的 i 使得 byteOffsets[i] <= byteOffset(即"落在该字符里")。
  byteToChar(byteOffset) {
    if (byteOffset <= 0) return 0;
    if (byteOffset >= this.totalBytes) return this.byteOffsets.length - 1;
    let lo = 0;
    let hi = this.byteOffsets.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >>> 1;
      if (this.byteOffsets[mid] <= byteOffset) lo = mid;
      else hi = mid - 1;
    }
    return lo;
  }

  /// 把核心库的 Span { start, end }(UTF-8 字节)转换为 UTF-16 区间。
  /// 返回 { start, end },可直接用作 String.prototype.slice 的参数。
  spanToCharRange(span) {
    return {
      start: this.byteToChar(span.start),
      end: this.byteToChar(span.end),
    };
  }
}
