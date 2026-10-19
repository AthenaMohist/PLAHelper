//! Bridge wire protocol — official Plasticity binary format (evidence-locked).
//!
//! Rewritten per PLA-004 G1 probe: the official protocol is **binary**, not
//! JSON. Evidence source (真源):
//!   docs/protocol-probe/capture-20261018/ (PROTOCOL-COMPARISON.md + frame-*.txt
//!   + bin/*.bin). Layout matches the C4D plugin reference implementation
//!   (protocol.py), byte-identical to the official blender addon.
//!
//! Wire format (little-endian throughout):
//!   header   : <u32 message_type><u32 message_id>          (8 bytes)
//!   string   : <u32 byte_len><utf8 bytes><pad to 4-byte alignment>
//!   error    : u32 code embedded in responses (200 = success)
//!   events   : u32 enum (NEW_VERSION_1=10, NEW_FILE_1=11, ...)
//!
//! MessageType table (confirmed by real-device HANDSHAKE supported-table echo):
//!   TRANSACTION_1=0 ADD_1=1 UPDATE_1=2 DELETE_1=3 MOVE_1=4 ATTRIBUTE_1=5
//!   NEW_VERSION_1=10 NEW_FILE_1=11 LIST_ALL_1=20 LIST_SOME_1=21
//!   LIST_VISIBLE_1=22 SUBSCRIBE_ALL_1=23 SUBSCRIBE_SOME_1=24
//!   UNSUBSCRIBE_ALL_1=25 REFACET_SOME_1=26 PUT_SOME_1=31 HANDSHAKE_1=100

use crate::error::BridgeError;

/// Known message type codes (full table, probe-confirmed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum MessageType {
    Transaction = 0,
    Add = 1,
    Update = 2,
    Delete = 3,
    Move = 4,
    Attribute = 5,
    NewVersion = 10,
    NewFile = 11,
    ListAll = 20,
    ListSome = 21,
    ListVisible = 22,
    SubscribeAll = 23,
    SubscribeSome = 24,
    UnsubscribeAll = 25,
    RefacetSome = 26,
    PutSome = 31,
    Handshake = 100,
    /// Unknown type code (forward compatibility; logged, never invented).
    Unknown(u32),
}

impl MessageType {
    pub fn from_u32(v: u32) -> Self {
        match v {
            0 => Self::Transaction,
            1 => Self::Add,
            2 => Self::Update,
            3 => Self::Delete,
            4 => Self::Move,
            5 => Self::Attribute,
            10 => Self::NewVersion,
            11 => Self::NewFile,
            20 => Self::ListAll,
            21 => Self::ListSome,
            22 => Self::ListVisible,
            23 => Self::SubscribeAll,
            24 => Self::SubscribeSome,
            25 => Self::UnsubscribeAll,
            26 => Self::RefacetSome,
            31 => Self::PutSome,
            100 => Self::Handshake,
            other => Self::Unknown(other),
        }
    }

    pub fn code(&self) -> u32 {
        match self {
            Self::Transaction => 0,
            Self::Add => 1,
            Self::Update => 2,
            Self::Delete => 3,
            Self::Move => 4,
            Self::Attribute => 5,
            Self::NewVersion => 10,
            Self::NewFile => 11,
            Self::ListAll => 20,
            Self::ListSome => 21,
            Self::ListVisible => 22,
            Self::SubscribeAll => 23,
            Self::SubscribeSome => 24,
            Self::UnsubscribeAll => 25,
            Self::RefacetSome => 26,
            Self::PutSome => 31,
            Self::Handshake => 100,
            Self::Unknown(v) => *v,
        }
    }
}

/// One decoded protocol frame: 8-byte header + type-specific payload.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub msg_type: MessageType,
    pub message_id: u32,
    pub payload: FramePayload,
}

/// Type-specific payload bodies. Only layouts with real-device evidence get
/// structured decoding; everything else stays raw (no guessing — G4 补采前
/// 按 ARCH §8.1 处理, 有参照源非 ASSUMED).
#[derive(Debug, Clone, PartialEq)]
pub enum FramePayload {
    /// HANDSHAKE_1 response: server's supported message-type table
    /// (real device echoed all 14 codes; frame-handshake-resp.bin, 68B).
    Handshake { supported: Vec<u32> },
    /// LIST_ALL/LIST_SOME/LIST_VISIBLE response:
    /// code(200) + filename str + version + object records (raw, see below).
    ListResponse {
        code: u32,
        filename: String,
        version: u32,
        /// Raw per-object records: u32 byte-length prefixed blobs.
        objects: Vec<Vec<u8>>,
    },
    /// PUT_SOME-style code responses (frame-putsome-resp-0.bin, 20B):
    /// code + trailing raw bytes (result tables).
    CodeResponse { code: u32, rest: Vec<u8> },
    /// Push events: raw body. NEW_VERSION/NEW_FILE layout is pending G4
    /// real-device sampling (type codes confirmed in the supported table;
    /// layout locked by reference implementation, not guessed).
    Event { body: Vec<u8> },
    /// Unrecognized payload kept verbatim (never dropped silently).
    Raw(Vec<u8>),
    /// Requests we send carry no payload (HANDSHAKE/LIST_ALL/SUBSCRIBE_ALL).
    Empty,
}

/// Explicit decode errors — truncated/overlong reads must error, never
/// silently short-read (ARCH §2 ProtocolError contract).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolError {
    pub op: &'static str,
    pub detail: String,
}

impl ProtocolError {
    fn new(op: &'static str, detail: impl Into<String>) -> Self {
        Self { op, detail: detail.into() }
    }
}

impl From<ProtocolError> for BridgeError {
    fn from(e: ProtocolError) -> Self {
        BridgeError::Codec(format!("{}: {}", e.op, e.detail))
    }
}

// ---- primitive encoders -----------------------------------------------------

pub fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

pub fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Length-prefixed string: u32 byte-length + UTF-8 + pad to 4-byte alignment.
/// (filename="model1.plasticity" 17B + 3 pad bytes, probe-confirmed)
pub fn put_string(out: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    put_u32(out, bytes.len() as u32);
    out.extend_from_slice(bytes);
    while out.len() % 4 != 0 {
        out.push(0);
    }
}

pub fn put_f32(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_le_bytes());
}

// ---- primitive decoders -----------------------------------------------------

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take(&mut self, n: usize, op: &'static str) -> Result<&'a [u8], ProtocolError> {
        if self.pos + n > self.buf.len() {
            return Err(ProtocolError::new(
                op,
                format!("truncated: need {} bytes at offset {}, have {}", n, self.pos, self.buf.len()),
            ));
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn u32(&mut self, op: &'static str) -> Result<u32, ProtocolError> {
        let b = self.take(4, op)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn string(&mut self, op: &'static str) -> Result<String, ProtocolError> {
        let len = self.u32(op)? as usize;
        let bytes = self.take(len, op)?;
        let s = std::str::from_utf8(bytes)
            .map_err(|e| ProtocolError::new(op, format!("invalid utf-8: {e}")))?
            .to_string();
        // alignment padding
        let pad = (-(self.pos as isize)).rem_euclid(4) as usize;
        self.take(pad, op)?;
        Ok(s)
    }

    fn rest(&self) -> &[u8] {
        &self.buf[self.pos.min(self.buf.len())..]
    }
}

// ---- frame encode / decode --------------------------------------------------

/// Encode a frame: 8-byte header + payload (ARCH §3 Encode contract).
pub fn encode(frame: &Frame) -> Result<Vec<u8>, ProtocolError> {
    let mut out = Vec::with_capacity(64);
    put_u32(&mut out, frame.msg_type.code());
    put_u32(&mut out, frame.message_id);
    match &frame.payload {
        FramePayload::Empty => {}
        FramePayload::Handshake { supported } => {
            put_u32(&mut out, supported.len() as u32);
            for c in supported {
                put_u32(&mut out, *c);
            }
        }
        FramePayload::ListResponse { code, filename, version, objects } => {
            put_u32(&mut out, *code);
            put_string(&mut out, filename);
            put_u32(&mut out, *version);
            put_u32(&mut out, objects.len() as u32);
            for obj in objects {
                put_u32(&mut out, obj.len() as u32);
                out.extend_from_slice(obj);
            }
        }
        FramePayload::CodeResponse { code, rest } => {
            put_u32(&mut out, *code);
            out.extend_from_slice(rest);
        }
        FramePayload::Event { body } => {
            out.extend_from_slice(body);
        }
        FramePayload::Raw(bytes) => {
            out.extend_from_slice(bytes);
        }
    }
    Ok(out)
}

/// Build a header-only request frame (HANDSHAKE_1 / LIST_ALL_1 / SUBSCRIBE_ALL_1).
pub fn request(msg_type: MessageType, message_id: u32) -> Frame {
    Frame { msg_type, message_id, payload: FramePayload::Empty }
}

/// Decode one binary frame (full WS message body, opcode=2).
pub fn decode(buf: &[u8]) -> Result<Frame, ProtocolError> {
    const OP: &str = "decode";
    let mut r = Reader::new(buf);
    let type_code = r.u32(OP)?;
    let message_id = r.u32(OP)?;
    let msg_type = MessageType::from_u32(type_code);

    let payload = match msg_type {
        MessageType::Handshake => {
            let n = r.u32(OP)? as usize;
            if n > 4096 {
                return Err(ProtocolError::new(OP, format!("supported table too large: {n}")));
            }
            let mut supported = Vec::with_capacity(n);
            for _ in 0..n {
                supported.push(r.u32(OP)?);
            }
            FramePayload::Handshake { supported }
        }
        MessageType::ListAll | MessageType::ListSome | MessageType::ListVisible => {
            let code = r.u32(OP)?;
            let filename = r.string(OP)?;
            let version = r.u32(OP)?;
            let n = r.u32(OP)? as usize;
            if n > 1_000_000 {
                return Err(ProtocolError::new(OP, format!("object count absurd: {n}")));
            }
            let mut objects = Vec::with_capacity(n.min(1024));
            for _ in 0..n {
                let len = r.u32(OP)? as usize;
                let body = r.take(len, OP).map_err(|_| {
                    // Wire-verified 2026-10-20: the device silently truncates
                    // oversized LIST_VISIBLE/LIST_ALL responses at ≈368KB —
                    // the last record's declared length exceeds the frame
                    // remainder. Fail loud with the measured numbers instead
                    // of serving a silently incomplete tree (R5 fix: 中文 +
                    // 声明/实收双数字, 直达 UI 错误浮窗; 实收 = 整帧字节数,
                    // 与 ≈368040B 单帧上限直接可比)。
                    ProtocolError::new(
                        OP,
                        format!(
                            "实体树帧被截断: 声明 {len}B 实收 {}B — 超过单帧上限, 需分片拉取(PLA-004 TODO#9)",
                            r.buf.len()
                        ),
                    )
                })?;
                objects.push(body.to_vec());
            }
            FramePayload::ListResponse { code, filename, version, objects }
        }
        MessageType::PutSome => {
            // PUT_SOME response (probe: 20B = mid + code + empty result table).
            let code = r.u32(OP)?;
            FramePayload::CodeResponse { code, rest: r.rest().to_vec() }
        }
        MessageType::NewVersion | MessageType::NewFile | MessageType::Transaction
        | MessageType::Add | MessageType::Update | MessageType::Delete | MessageType::Move
        | MessageType::Attribute => FramePayload::Event { body: r.rest().to_vec() },
        MessageType::SubscribeAll | MessageType::SubscribeSome | MessageType::UnsubscribeAll
        | MessageType::RefacetSome => FramePayload::Raw(r.rest().to_vec()),
        MessageType::Unknown(_) => FramePayload::Raw(r.rest().to_vec()),
    };
    Ok(Frame { msg_type, message_id, payload })
}

/// Extract server result code from a code-bearing response (PUT_SOME etc.).
pub fn response_code(frame: &Frame) -> Option<u32> {
    match &frame.payload {
        FramePayload::CodeResponse { code, .. } => Some(*code),
        FramePayload::ListResponse { code, .. } => Some(*code),
        _ => None,
    }
}

/// A successful LIST_ALL response decoded into a JSON mirror.
///
/// The JSON shape mirrors the binary fields 1:1 (code/filename/version/objects)
/// so callers keep a single representation without inventing a second schema.
/// Object records stay opaque (`objects[].raw_len` only) until a caller needs
/// the geometry stream (G4 scope; ARCH §8.1).
/// Parse one LIST_ALL object record. [DPL-218 逆向 2026-10-18: 真机 44B Scene + 1352B Extruded.002 逐字节对照]
///   +00 flags(=1) | +04 kind_raw(1=Scene,4=Extruded) | +08 type_code(0=scene,5=solid)
///   +0c index(实体编号=对外 id) | +10/+14 opaque | +18 parent(0xffffffff=根)
///   +1c tag=6 | +20 strlen | name utf8 + 4B 对齐 pad | 之后 [blk_len][blk] 变长块(bbox 72B 等)
/// facets: 真机变长块语义未实锤 → 返回空, dim 顶点平移待后续探针。

/// 单个实体名块(R7-12 逆向实锤, 双帧 17/17 记录验证)。
///
/// 码 20/22 响应的记录体内是背靠背的名块流:
///   `[T][idx][a][b][parent=0xffffffff][tag=6][slen][name utf8+pad]`
///   * T=type_code: **0=实体(solid) 1=片体(sheet) 5=组/容器(Scene/Group)**
///     (旧 DPL-218 的 kind_raw=4=Extruded 系把运行计数器误当类型 — 4/14/232
///     等值随文档变化, 已证伪);
///   * idx=树内编号, a=全局实体号(文档内唯一, 作对外 id), b=父节点 idx
///     (Scene=0; 实测 '111' b=0 / '111222' b=1 / 部分体 b=1、b=2 与
///     Scene→'111'→'111222' 三层嵌套完全吻合);
///   * parent 槽恒 0xffffffff(根标记), 真层级在 b。
/// 体记录(T=0/1)名后紧跟 mesh: [nverts][verts nv*24B][ntri][tris ntri*12B]
/// [nn(=nv)][normals nn*12B][尾部块(96B 级, 结构未解)] — 前五段可精确跳过。
#[derive(Debug, Clone)]
pub struct EntityBlock {
    /// type_code: 0=solid, 1=sheet, 5=group/scene。
    pub tcode: u32,
    /// 树内编号(父引用 b 指向它)。
    pub idx: u32,
    /// 全局实体号(文档内唯一) — 对外 id。
    pub a: u32,
    /// 父节点 idx(Scene=0), 0xffffffff 视为根。
    pub b: u32,
    pub name: String,
    /// 网格顶点(真机 mesh verts / mock facet verts) — dim 平移用。
    pub facets: Vec<Vec<[f32; 3]>>,
}

/// 在记录体内扫描实体名块。带 T/idx/a/b 合法性校验 + 体记录 mesh 区段
/// 精确跳过(verts/normals 浮点海是误报重灾区, 跳过即归零)。
fn scan_entity_blocks(rec: &[u8]) -> Vec<EntityBlock> {
    let mut out = Vec::new();
    if rec.len() < 28 { return out; }
    let mut pos = 0usize;
    // 循环条件只保证 tag+slen 在界内(名块可贴记录尾); header 回看由 pos>=20 保证
    while pos + 8 <= rec.len() {
        // 名块锚: [tag=6][slen 1..48][printable name]
        if u32::from_le_bytes(rec[pos..pos+4].try_into().unwrap()) != 6 {
            pos += 4;
            continue;
        }
        let slen = u32::from_le_bytes(rec[pos+4..pos+8].try_into().unwrap()) as usize;
        if !(1..=48).contains(&slen) || pos + 8 + slen > rec.len() {
            pos += 4;
            continue;
        }
        let nm = &rec[pos+8..pos+8+slen];
        if !nm.iter().all(|c| (32..127).contains(c)) {
            pos += 4;
            continue;
        }
        // 5 字段头在 tag 前 20B
        if pos < 20 {
            pos += 4;
            continue;
        }
        let h = struct_u5(rec, pos - 20);
        let (tcode, idx, a, b, parent) = h;
        let hdr_ok = parent == 0xffff_ffff
            && tcode <= 64
            && idx < 1_000_000
            && a < 1_000_000
            && b < 1_000_000;
        if !hdr_ok {
            pos += 4;
            continue;
        }
        let mut next = pos + 8 + slen;
        next += (4 - (next % 4)) % 4;
        // payload 双格式解析(R7-18): 体记录名后是真 mesh
        // [nv][verts nv*12B][ntri][tris ntri*12B][nn(==nv)][normals nn*12B][尾部未解],
        // mock 是 legacy facets [fcount≤64][每 facet: vcount][verts vcount*12B]。
        // 真格式优先(verts 即 dim 所需顶点), 校验失败退 legacy, 再失败小步扫描。
        let mut facets: Vec<Vec<[f32; 3]>> = Vec::new();
        // payload 解析仅对体记录(T=0/1): T=5 组的 payload 是内联子记录,
        // 不能当 facet/numeric 头解析(否则 fcount 误读会吞掉后续记录)。
        if (tcode == 0 || tcode == 1) && next + 4 <= rec.len() {
            let n0 = u32::from_le_bytes(rec[next..next+4].try_into().unwrap()) as usize;
            let mut ok = false;
            if (1..=10_000_000).contains(&n0) {
                let mut p = next + 4 + n0 * 12;
                if p + 4 <= rec.len() {
                    let ntri = u32::from_le_bytes(rec[p..p+4].try_into().unwrap()) as usize;
                    p += 4 + ntri.saturating_mul(12);
                    if (0..=10_000_000).contains(&ntri) && p + 4 <= rec.len() {
                        let nn = u32::from_le_bytes(rec[p..p+4].try_into().unwrap()) as usize;
                        p += 4 + nn.saturating_mul(12);
                        if nn == n0 && p <= rec.len() {
                            ok = true;
                            let mut verts = Vec::with_capacity(n0);
                            for i in 0..n0 {
                                let o = next + 4 + i * 12;
                                let x = f32::from_le_bytes(rec[o..o+4].try_into().unwrap());
                                let y = f32::from_le_bytes(rec[o+4..o+8].try_into().unwrap());
                                let z = f32::from_le_bytes(rec[o+8..o+12].try_into().unwrap());
                                verts.push([x, y, z]);
                            }
                            facets.push(verts);
                            next = p;
                        }
                    }
                }
            }
            if !ok {
                let mut p = next;
                let mut ok2 = true;
                let mut f: Vec<Vec<[f32; 3]>> = Vec::new();
                if p + 4 <= rec.len() {
                    let fcount = u32::from_le_bytes(rec[p..p+4].try_into().unwrap()) as usize;
                    p += 4;
                    if (1..=64).contains(&fcount) {
                        for _ in 0..fcount {
                            if p + 4 > rec.len() { ok2 = false; break }
                            let vc = u32::from_le_bytes(rec[p..p+4].try_into().unwrap()) as usize;
                            p += 4;
                            if vc > 65536 || p + vc * 12 > rec.len() { ok2 = false; break }
                            let mut verts = Vec::with_capacity(vc);
                            for i in 0..vc {
                                let o = p + i * 12;
                                let x = f32::from_le_bytes(rec[o..o+4].try_into().unwrap());
                                let y = f32::from_le_bytes(rec[o+4..o+8].try_into().unwrap());
                                let z = f32::from_le_bytes(rec[o+8..o+12].try_into().unwrap());
                                verts.push([x, y, z]);
                            }
                            p += vc * 12;
                            f.push(verts);
                        }
                        if ok2 {
                            facets = f;
                            next = p;
                        }
                    }
                }
            }
        }
        out.push(EntityBlock {
            tcode, idx, a, b, name: String::from_utf8_lossy(nm).to_string(),
            facets,
        });
        pos = next;
    }
    out
}

fn struct_u5(rec: &[u8], at: usize) -> (u32, u32, u32, u32, u32) {
    (
        u32::from_le_bytes(rec[at..at+4].try_into().unwrap()),
        u32::from_le_bytes(rec[at+4..at+8].try_into().unwrap()),
        u32::from_le_bytes(rec[at+8..at+12].try_into().unwrap()),
        u32::from_le_bytes(rec[at+12..at+16].try_into().unwrap()),
        u32::from_le_bytes(rec[at+16..at+20].try_into().unwrap()),
    )
}

fn kind_name(tcode: u32) -> String {
    match tcode {
        0 => "solid".to_string(),
        1 => "sheet".to_string(),
        5 => "group".to_string(),
        // TODO-PROBE: 真机 fillet 的 T 码未实锤(R7-18); 9 沿用 mock 约定,
        // 维持 pla-dim 拓扑硬拒(TOPOLOGY_KINDS)的测试路径。真机码探明后更新。
        9 => "fillet".to_string(),
        other => format!("type{other}"),
    }
}

pub fn list_response_to_value(frame: &Frame) -> Result<serde_json::Value, BridgeError> {
    match &frame.payload {
        FramePayload::ListResponse { code, filename, version, objects } => {
            let mut objs: Vec<serde_json::Value> = Vec::new();
            for body in objects {
                for blk in scan_entity_blocks(body) {
                    let fj: Vec<serde_json::Value> = blk
                        .facets
                        .iter()
                        .map(|verts| {
                            serde_json::json!({ "vertices": verts.iter()
                                .map(|v| serde_json::json!([v[0], v[1], v[2]]))
                                .collect::<Vec<_>>() })
                        })
                        .collect();
                    objs.push(serde_json::json!({
                        // idx = 树内编号, 全帧唯一(实测 0..24 无重复) — 对外 id。
                        // (a 是分类型计数器, group 与 solid 间会撞号, 不能作 id)
                        "id": blk.idx.to_string(),
                        "order": blk.a,
                        "parent_idx": blk.b,
                        "kind": kind_name(blk.tcode),
                        "name": blk.name,
                        "facets": fj,
                        "raw_len": body.len(),
                    }));
                }
            }
            Ok(serde_json::json!({
                "code": code,
                "filename": filename,
                "version": version,
                // 键名 "entities" 对齐 pla_selection::entity_tree_from_payload 的消费(DPL-218 OC 直修实证: 原 "objects" 上层永远读空)
                "entities": objs,
            }))
        }
        _ => Err(BridgeError::Codec("not a ListResponse frame".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real captured HANDSHAKE response (68 bytes, bin/handshake-resp.bin).
    const HANDSHAKE_RESP: &[u8] = &[
        0x64, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x0e, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00,
        0x03, 0x00, 0x00, 0x00, 0x0a, 0x00, 0x00, 0x00, 0x0b, 0x00, 0x00, 0x00,
        0x14, 0x00, 0x00, 0x00, 0x16, 0x00, 0x00, 0x00, 0x17, 0x00, 0x00, 0x00,
        0x18, 0x00, 0x00, 0x00, 0x19, 0x00, 0x00, 0x00, 0x1a, 0x00, 0x00, 0x00,
        0x1f, 0x00, 0x00, 0x00, 0x64, 0x00, 0x00, 0x00,
    ];

    /// Real captured PUT_SOME response (20 bytes, bin/putsome-resp-0.bin).
    const PUTSOME_RESP: &[u8] = &[
        0x1f, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0xc8, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    /// First 64 bytes of the real LIST_ALL response (bin/listall-resp-0.bin).
    const LISTALL_HEAD: &[u8] = &[
        0x14, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0xc8, 0x00, 0x00, 0x00,
        0x11, 0x00, 0x00, 0x00, 0x6d, 0x6f, 0x64, 0x65, 0x6c, 0x31, 0x2e, 0x70,
        0x6c, 0x61, 0x73, 0x74, 0x69, 0x63, 0x69, 0x74, 0x79, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00,
        0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn decode_real_handshake_response() {
        let f = decode(HANDSHAKE_RESP).expect("decode handshake");
        assert_eq!(f.msg_type, MessageType::Handshake);
        assert_eq!(f.message_id, 1);
        match &f.payload {
            FramePayload::Handshake { supported } => {
                assert_eq!(supported.len(), 14);
                assert_eq!(supported[0], 0); // TRANSACTION_1
                assert!(supported.contains(&20)); // LIST_ALL_1
                assert!(supported.contains(&31)); // PUT_SOME_1
                assert!(supported.contains(&100)); // HANDSHAKE_1
            }
            other => panic!("wrong payload: {other:?}"),
        }
        // byte-level roundtrip: encode(decode(x)) == x
        let re = encode(&f).expect("encode");
        assert_eq!(re, HANDSHAKE_RESP);
    }

    #[test]
    fn decode_real_putsome_response() {
        let f = decode(PUTSOME_RESP).expect("decode putsome");
        assert_eq!(f.msg_type, MessageType::PutSome);
        assert_eq!(f.message_id, 4);
        match &f.payload {
            FramePayload::CodeResponse { code, rest } => {
                assert_eq!(*code, 200);
                assert_eq!(rest.len(), 8); // empty result tables
            }
            other => panic!("wrong payload: {other:?}"),
        }
        let re = encode(&f).expect("encode");
        assert_eq!(re, PUTSOME_RESP);
    }

    #[test]
    fn decode_listall_head_fields() {
        // 64B truncated head: header + code + filename + version + count +
        // first record length (44B) — record body truncated on purpose.
        let f = decode(LISTALL_HEAD);
        // Truncated record body must error explicitly (no silent short read).
        assert!(f.is_err(), "truncated frame must error");
        let err = f.unwrap_err();
        assert_eq!(err.op, "decode");
        // R5: 截断错误必须带声明/实收双数字(fail-loud 直达 UI 错误浮窗);
        // 实收 = 整帧 64B(LISTALL_HEAD 全长)。
        assert!(err.detail.contains("截断"), "got: {}", err.detail);
        assert!(err.detail.contains("声明 44B"), "got: {}", err.detail);
        assert!(err.detail.contains("实收 64B"), "got: {}", err.detail);

        // With the first record body appended (zeros), decode completes both
        // records: the appended zeros parse as an empty second record.
        let mut full = LISTALL_HEAD.to_vec();
        full.extend_from_slice(&[0u8; 44]);
        let f2 = decode(&full).expect("head + one record body parses");
        match f2.payload {
            FramePayload::ListResponse { objects, .. } => {
                assert_eq!(objects.len(), 2);
                assert_eq!(objects[0].len(), 44);
                assert_eq!(objects[1].len(), 0);
            }
            other => panic!("wrong payload: {other:?}"),
        }

        // Full first-record decode verified field-by-field via a hand-built
        // single-object frame with the same probe header fields.
        let mut single = Vec::new();
        put_u32(&mut single, MessageType::ListAll.code());
        put_u32(&mut single, 2);
        put_u32(&mut single, 200);
        put_string(&mut single, "model1.plasticity");
        put_u32(&mut single, 0);
        put_u32(&mut single, 1);
        put_u32(&mut single, 44);
        single.extend_from_slice(&[0u8; 44]);
        let f = decode(&single).expect("decode single-object frame");
        assert_eq!(f.msg_type, MessageType::ListAll);
        assert_eq!(f.message_id, 2);
        match &f.payload {
            FramePayload::ListResponse { code, filename, version, objects } => {
                assert_eq!(*code, 200);
                assert_eq!(filename, "model1.plasticity");
                assert_eq!(*version, 0);
                assert_eq!(objects.len(), 1);
                assert_eq!(objects[0].len(), 44);
            }
            other => panic!("wrong payload: {other:?}"),
        }
        let re = encode(&f).expect("encode");
        assert_eq!(re, single);
    }

    #[test]
    fn request_header_layout() {
        let hs = encode(&request(MessageType::Handshake, 1)).expect("encode");
        assert_eq!(hs, [100, 0, 0, 0, 1, 0, 0, 0]);
        let la = encode(&request(MessageType::ListAll, 2)).expect("encode");
        assert_eq!(la, [20, 0, 0, 0, 2, 0, 0, 0]);
    }

    #[test]
    fn string_padding_matches_probe() {
        // "model1.plasticity" = 4B length prefix + 17 bytes + 3 pad = 24 bytes.
        let mut out = Vec::new();
        put_string(&mut out, "model1.plasticity");
        assert_eq!(out.len(), 24);
        assert_eq!(&out[..4], &[17, 0, 0, 0]);
        assert_eq!(&out[4..21], b"model1.plasticity");
        assert!(out[21..].iter().all(|&b| b == 0));
    }

    #[test]
    fn unknown_type_preserved_verbatim() {
        let raw = [0xde, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0xaa, 0xbb];
        let f = decode(&raw).expect("decode unknown");
        assert_eq!(f.msg_type, MessageType::Unknown(0xde));
        let re = encode(&f).expect("encode");
        assert_eq!(re, raw);
    }

    #[test]
    fn error_response_maps_to_server_code() {
        // code != 200 must surface as BridgeError, not panic (US3 TC-C2-1).
        let raw = [
            0x1f, 0x00, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00, 0x32, 0x00, 0x00, 0x00,
        ];
        let f = decode(&raw).expect("decode");
        match f.payload {
            FramePayload::CodeResponse { code, .. } => {
                assert_eq!(code, 50);
                let err: BridgeError =
                    BridgeError::ServerCode { op: "PUT_SOME", code }.into();
                assert!(err.to_string().contains("50"));
            }
            other => panic!("wrong payload: {other:?}"),
        }
    }

// [OC 临时实证] 用 dim mock 的字节验证 decode
#[test]
fn oc_decode_mock_listall() {
    let mut v = Vec::new();
    v.extend_from_slice(&20u32.to_le_bytes());
    v.extend_from_slice(&1u32.to_le_bytes());
    v.extend_from_slice(&200u32.to_le_bytes());
    let fname = b"model1.plasticity";
    v.extend_from_slice(&(fname.len() as u32).to_le_bytes());
    v.extend_from_slice(fname);
    let pad = (4 - (fname.len() % 4)) % 4;
    v.extend_from_slice(&vec![0u8; pad]);
    v.extend_from_slice(&0u32.to_le_bytes());
    let mut rec = Vec::new();
    rec.extend_from_slice(&1u32.to_le_bytes());
    rec.extend_from_slice(&1u32.to_le_bytes());
    rec.extend_from_slice(&4u32.to_le_bytes());
    rec.extend_from_slice(b"Body");
    rec.extend_from_slice(&vec![0u8; 0]);
    v.extend_from_slice(&1u32.to_le_bytes());
    v.extend_from_slice(&(rec.len() as u32).to_le_bytes());
    v.extend_from_slice(&rec);
    let f = decode(&v).expect("decode mock frame");
    match f.payload {
        FramePayload::ListResponse { code, ref filename, objects, .. } => {
            assert_eq!(code, 200);
            assert_eq!(filename, "model1.plasticity");
            assert_eq!(objects.len(), 1);
        }
        other => panic!("wrong payload: {other:?}"),
    }
}
}

#[cfg(test)]
mod r712_tests {
    use super::*;

    fn block(idx: u32, a: u32, name: &[u8]) -> Vec<u8> {
        let mut b = Vec::new();
        for v in [5u32, idx, a, 0, 0xffff_ffff] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&6u32.to_le_bytes());
        b.extend_from_slice(&(name.len() as u32).to_le_bytes());
        b.extend_from_slice(name);
        while b.len() % 4 != 0 { b.push(0); }
        b
    }

    #[test]
    fn scan_finds_two_adjacent_group_blocks() {
        let mut body = block(0, 100, b"e0");
        body.extend_from_slice(&block(1, 101, b"e1"));
        let blks = scan_entity_blocks(&body);
        assert_eq!(blks.len(), 2, "got: {blks:?}");
        assert_eq!(blks[0].name, "e0");
        assert_eq!(blks[1].name, "e1");
        assert_eq!(blks[1].a, 101);
    }

    #[test]
    fn scan_mesh_body_skips_floats() {
        // T=0 体块 + nverts=2 + verts/normals 浮点 + ntri=1 + 3 索引 + 尾巴
        let mut body = block(0, 1, b"Solid.001");
        let mut mesh = Vec::new();
        mesh.extend_from_slice(&2u32.to_le_bytes()); // nverts
        mesh.extend_from_slice(&[0u8; 48]);          // verts 2*24
        mesh.extend_from_slice(&1u32.to_le_bytes()); // ntri
        mesh.extend_from_slice(&[0u8; 12]);          // tris
        mesh.extend_from_slice(&2u32.to_le_bytes()); // nn
        mesh.extend_from_slice(&[0u8; 48]);          // normals
        mesh.extend_from_slice(&[0x06, 0, 0, 0, 3, 0, 0, 0]); // 尾部伪装名块(应被 T 校验拦下? T=3 合法… 此处只验不 panic)
        body.extend_from_slice(&mesh);
        body.extend_from_slice(&block(1, 2, b"Next"));
        let blks = scan_entity_blocks(&body);
        assert_eq!(blks.len(), 2, "got: {blks:?}");
        assert_eq!(blks[1].name, "Next");
    }
}
