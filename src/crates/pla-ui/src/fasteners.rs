//! A10 特征宏 + N5 国标紧固件目录: 国标标准件 → 建模步骤(垫片 `cmd.exec` 的 ops: revolve / prism / cylinder)。
//! 用户 2026-11-17 选题: 先做螺丝, 另加轴类与其它标准件; 2026-11-18: 打孔面板「放螺丝」要能选国标各种紧固件(含各种头型)、
//! 垫圈 / 弹垫 / 螺母, 尺寸一律以这里的表为准(不再靠 asset 里的负体记孔径)。
//!
//! 尺寸取各标准的公称 / 最大值(建模用, 不含公差): 头径 dk、头高 k、内六角对边 s、内六角深 t、六角对边 s、螺母厚 m、垫圈 d1/d2/h;
//! 端部倒角 ≈ 0.6 × 粗牙螺距(45°)。螺纹不建(外观 / 出图标注另做); 十字槽 / 一字槽暂不建(建库时再补)。
//! 长度范围 lmin..lmax = 该规格在标准里的商品长度范围(取 [`L_SERIES`] 里落在范围内的)。
//!
//! 放置: `at` + `axis`。螺钉 / 螺栓: at = 头的支承面中心(沉头 = 头顶面中心), axis = 从头指向杆端(插进孔的方向);
//! 紧定螺钉: at = 有内六角的端面中心, axis 指向另一端; 螺母 / 垫圈 / 销 / 轴: at = 一个端面中心, axis = 厚度 / 长度方向。

use serde_json::{json, Value};

/// 粗牙螺距(GB/T 193)。
const PITCH: &[(f64, f64)] = &[
    (1.6, 0.35), (2.0, 0.4), (2.5, 0.45), (3.0, 0.5), (4.0, 0.7), (5.0, 0.8), (6.0, 1.0), (8.0, 1.25), (10.0, 1.5), (12.0, 1.75), (14.0, 2.0), (16.0, 2.0),
    (20.0, 2.5), (24.0, 3.0),
];

/// 螺钉 / 螺栓的公称长度系列(各标准的优选长度合并; 某规格可用的 = 落在它 lmin..lmax 里的)。
pub const L_SERIES: &[f64] = &[
    2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0, 16.0, 20.0, 25.0, 30.0, 35.0, 40.0, 45.0, 50.0, 55.0, 60.0, 65.0, 70.0, 80.0, 90.0, 100.0, 110.0, 120.0,
    130.0, 140.0, 150.0, 160.0, 180.0, 200.0, 220.0, 240.0,
];

/// 头型(决定建模轮廓与长度怎么算)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Head {
    /// 圆柱头(内六角)
    Cap,
    /// 半圆头(内六角)
    Button,
    /// 盘头
    Pan,
    /// 开槽圆柱头
    Cheese,
    /// 沉头(90°; 公称长度含头, 头顶与面平)
    Csk,
    /// 六角头
    Hex,
    /// 紧定螺钉(无头)
    Set,
}

/// 驱动方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drive {
    Socket,
    Cross,
    Slot,
    /// 外六角(扳手)
    Outer,
}

/// 一个规格: d, 头径 dk(六角头 = 对边 s), 头高 k(沉头 = 锥高), 内六角对边 s / 深 t(没有 = 0), 长度范围。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Row {
    pub d: f64,
    pub dk: f64,
    pub k: f64,
    pub s: f64,
    pub t: f64,
    pub lmin: f64,
    pub lmax: f64,
}

const fn r(d: f64, dk: f64, k: f64, s: f64, t: f64, lmin: f64, lmax: f64) -> Row {
    Row { d, dk, k, s, t, lmin, lmax }
}

/// 一种螺钉 / 螺栓标准。`aliases` = 认 asset 名字 / 文件夹用的俗称。
#[derive(Debug)]
pub struct ScrewStd {
    pub id: &'static str,
    pub code: &'static str,
    pub zh: &'static str,
    pub en: &'static str,
    pub head: Head,
    pub drive: Drive,
    pub aliases: &'static [&'static str],
    pub rows: &'static [Row],
}

/// 头下的沉头规格(GB/T 819.1 / GB/T 68 相同)。
const CSK_CROSS: &[Row] = &[
    r(1.6, 3.6, 1.0, 0.0, 0.0, 3.0, 16.0), r(2.0, 4.4, 1.2, 0.0, 0.0, 3.0, 20.0), r(2.5, 5.5, 1.5, 0.0, 0.0, 3.0, 25.0), r(3.0, 6.3, 1.65, 0.0, 0.0, 4.0, 30.0),
    r(4.0, 9.4, 2.7, 0.0, 0.0, 5.0, 40.0), r(5.0, 10.4, 2.7, 0.0, 0.0, 6.0, 50.0), r(6.0, 12.6, 3.3, 0.0, 0.0, 8.0, 60.0), r(8.0, 17.3, 4.65, 0.0, 0.0, 10.0, 60.0),
    r(10.0, 20.0, 5.0, 0.0, 0.0, 12.0, 60.0),
];
const HEX_HEAD: &[Row] = &[
    r(3.0, 5.5, 2.0, 0.0, 0.0, 6.0, 30.0), r(4.0, 7.0, 2.8, 0.0, 0.0, 8.0, 40.0), r(5.0, 8.0, 3.5, 0.0, 0.0, 10.0, 50.0), r(6.0, 10.0, 4.0, 0.0, 0.0, 12.0, 60.0),
    r(8.0, 13.0, 5.3, 0.0, 0.0, 16.0, 80.0), r(10.0, 16.0, 6.4, 0.0, 0.0, 20.0, 100.0), r(12.0, 18.0, 7.5, 0.0, 0.0, 25.0, 120.0), r(14.0, 21.0, 8.8, 0.0, 0.0, 30.0, 140.0),
    r(16.0, 24.0, 10.0, 0.0, 0.0, 30.0, 150.0), r(20.0, 30.0, 12.5, 0.0, 0.0, 40.0, 150.0), r(24.0, 36.0, 15.0, 0.0, 0.0, 50.0, 150.0),
];
const SET_ROWS: &[Row] = &[
    r(3.0, 3.0, 0.0, 1.5, 1.2, 3.0, 16.0), r(4.0, 4.0, 0.0, 2.0, 1.5, 4.0, 20.0), r(5.0, 5.0, 0.0, 2.5, 2.0, 5.0, 25.0), r(6.0, 6.0, 0.0, 3.0, 2.0, 6.0, 30.0),
    r(8.0, 8.0, 0.0, 4.0, 3.0, 8.0, 40.0), r(10.0, 10.0, 0.0, 5.0, 4.0, 10.0, 50.0), r(12.0, 12.0, 0.0, 6.0, 4.8, 12.0, 60.0), r(16.0, 16.0, 0.0, 8.0, 6.4, 16.0, 60.0),
];

/// 国标螺钉 / 螺栓目录(打孔面板第一行的"标准"下拉, 顺序即显示顺序)。
pub const SCREWS: &[ScrewStd] = &[
    ScrewStd {
        id: "gb70.1", code: "GB/T 70.1", zh: "内六角圆柱头螺钉", en: "Socket head cap screw", head: Head::Cap, drive: Drive::Socket,
        aliases: &["70.1", "内六角圆柱头", "杯头内六角", "杯内六", "杯头", "socket head cap", "shcs"],
        rows: &[
            r(1.6, 3.0, 1.6, 1.5, 0.7, 2.5, 16.0), r(2.0, 3.8, 2.0, 1.5, 1.0, 3.0, 20.0), r(2.5, 4.5, 2.5, 2.0, 1.1, 4.0, 25.0), r(3.0, 5.5, 3.0, 2.5, 1.3, 5.0, 30.0),
            r(4.0, 7.0, 4.0, 3.0, 2.0, 6.0, 40.0), r(5.0, 8.5, 5.0, 4.0, 2.5, 8.0, 50.0), r(6.0, 10.0, 6.0, 5.0, 3.0, 10.0, 60.0), r(8.0, 13.0, 8.0, 6.0, 4.0, 12.0, 80.0),
            r(10.0, 16.0, 10.0, 8.0, 5.0, 16.0, 100.0), r(12.0, 18.0, 12.0, 10.0, 6.0, 20.0, 120.0), r(14.0, 21.0, 14.0, 12.0, 7.0, 25.0, 140.0),
            r(16.0, 24.0, 16.0, 14.0, 8.0, 25.0, 160.0), r(20.0, 30.0, 20.0, 17.0, 10.0, 30.0, 200.0), r(24.0, 36.0, 24.0, 19.0, 12.0, 40.0, 200.0),
        ],
    },
    ScrewStd {
        id: "gb70.2", code: "GB/T 70.2", zh: "内六角半圆头螺钉", en: "Socket button head screw", head: Head::Button, drive: Drive::Socket,
        aliases: &["70.2", "半圆头内六角", "圆杯", "button"],
        rows: &[
            r(3.0, 5.7, 1.65, 2.0, 1.04, 6.0, 12.0), r(4.0, 7.6, 2.2, 2.5, 1.3, 6.0, 20.0), r(5.0, 9.5, 2.75, 3.0, 1.56, 8.0, 30.0), r(6.0, 10.5, 3.3, 4.0, 2.08, 10.0, 30.0),
            r(8.0, 14.0, 4.4, 5.0, 2.6, 10.0, 40.0), r(10.0, 17.5, 5.5, 6.0, 3.12, 16.0, 40.0), r(12.0, 21.0, 6.6, 8.0, 4.16, 16.0, 50.0), r(16.0, 28.0, 8.8, 10.0, 5.2, 30.0, 60.0),
        ],
    },
    ScrewStd {
        id: "gb70.3", code: "GB/T 70.3", zh: "内六角沉头螺钉", en: "Socket countersunk screw", head: Head::Csk, drive: Drive::Socket,
        aliases: &["70.3", "内六角沉头", "沉头内六角", "平杯", "countersunk socket"],
        rows: &[
            r(3.0, 6.72, 1.86, 2.0, 1.1, 8.0, 30.0), r(4.0, 8.96, 2.48, 2.5, 1.5, 8.0, 40.0), r(5.0, 11.2, 3.1, 3.0, 1.9, 8.0, 50.0), r(6.0, 13.44, 3.72, 4.0, 2.2, 8.0, 60.0),
            r(8.0, 17.92, 4.96, 5.0, 3.0, 10.0, 80.0), r(10.0, 22.4, 6.2, 6.0, 3.6, 12.0, 100.0), r(12.0, 26.88, 7.44, 8.0, 4.3, 20.0, 100.0),
            r(16.0, 33.6, 8.8, 10.0, 5.6, 30.0, 100.0), r(20.0, 40.32, 10.16, 12.0, 6.6, 35.0, 100.0),
        ],
    },
    ScrewStd {
        id: "gb5783", code: "GB/T 5783", zh: "六角头螺栓(全螺纹)", en: "Hex head bolt, full thread", head: Head::Hex, drive: Drive::Outer,
        aliases: &["5783", "外六角", "六角头螺栓", "hex bolt"],
        rows: HEX_HEAD,
    },
    ScrewStd {
        id: "gb5782", code: "GB/T 5782", zh: "六角头螺栓(半螺纹)", en: "Hex head bolt, partial thread", head: Head::Hex, drive: Drive::Outer,
        aliases: &["5782", "半牙外六角"],
        rows: &[
            r(5.0, 8.0, 3.5, 0.0, 0.0, 25.0, 50.0), r(6.0, 10.0, 4.0, 0.0, 0.0, 30.0, 60.0), r(8.0, 13.0, 5.3, 0.0, 0.0, 35.0, 80.0), r(10.0, 16.0, 6.4, 0.0, 0.0, 40.0, 100.0),
            r(12.0, 18.0, 7.5, 0.0, 0.0, 45.0, 120.0), r(14.0, 21.0, 8.8, 0.0, 0.0, 50.0, 140.0), r(16.0, 24.0, 10.0, 0.0, 0.0, 55.0, 160.0),
            r(20.0, 30.0, 12.5, 0.0, 0.0, 65.0, 200.0), r(24.0, 36.0, 15.0, 0.0, 0.0, 80.0, 240.0),
        ],
    },
    ScrewStd {
        id: "gb818", code: "GB/T 818", zh: "十字槽盘头螺钉", en: "Cross recessed pan head screw", head: Head::Pan, drive: Drive::Cross,
        aliases: &["818", "十字盘头", "盘头十字", "pan head cross"],
        rows: &[
            r(1.6, 3.2, 1.3, 0.0, 0.0, 3.0, 16.0), r(2.0, 4.0, 1.6, 0.0, 0.0, 3.0, 20.0), r(2.5, 5.0, 2.1, 0.0, 0.0, 3.0, 25.0), r(3.0, 5.6, 2.4, 0.0, 0.0, 4.0, 30.0),
            r(4.0, 8.0, 3.1, 0.0, 0.0, 5.0, 40.0), r(5.0, 9.5, 3.7, 0.0, 0.0, 6.0, 45.0), r(6.0, 12.0, 4.6, 0.0, 0.0, 8.0, 60.0), r(8.0, 16.0, 6.0, 0.0, 0.0, 10.0, 60.0),
            r(10.0, 20.0, 7.5, 0.0, 0.0, 12.0, 60.0),
        ],
    },
    ScrewStd {
        id: "gb819.1", code: "GB/T 819.1", zh: "十字槽沉头螺钉", en: "Cross recessed countersunk screw", head: Head::Csk, drive: Drive::Cross,
        aliases: &["819", "十字沉头", "沉头十字", "countersunk cross"],
        rows: CSK_CROSS,
    },
    ScrewStd {
        id: "gb65", code: "GB/T 65", zh: "开槽圆柱头螺钉", en: "Slotted cheese head screw", head: Head::Cheese, drive: Drive::Slot,
        aliases: &["gb65", "GB/T 65", "开槽圆柱头", "一字圆柱头", "cheese head"],
        rows: &[
            r(1.6, 3.0, 1.1, 0.0, 0.0, 2.0, 16.0), r(2.0, 3.8, 1.4, 0.0, 0.0, 3.0, 20.0), r(2.5, 4.5, 1.8, 0.0, 0.0, 3.0, 25.0), r(3.0, 5.5, 2.0, 0.0, 0.0, 4.0, 30.0),
            r(4.0, 7.0, 2.6, 0.0, 0.0, 5.0, 40.0), r(5.0, 8.5, 3.3, 0.0, 0.0, 6.0, 50.0), r(6.0, 10.0, 3.9, 0.0, 0.0, 8.0, 60.0), r(8.0, 13.0, 5.0, 0.0, 0.0, 10.0, 80.0),
            r(10.0, 16.0, 6.0, 0.0, 0.0, 12.0, 80.0),
        ],
    },
    ScrewStd {
        id: "gb67", code: "GB/T 67", zh: "开槽盘头螺钉", en: "Slotted pan head screw", head: Head::Pan, drive: Drive::Slot,
        aliases: &["gb67", "GB/T 67", "开槽盘头", "一字盘头", "slotted pan"],
        rows: &[
            r(1.6, 3.2, 1.0, 0.0, 0.0, 2.0, 16.0), r(2.0, 4.0, 1.3, 0.0, 0.0, 2.5, 20.0), r(2.5, 5.0, 1.5, 0.0, 0.0, 3.0, 25.0), r(3.0, 5.6, 1.8, 0.0, 0.0, 4.0, 30.0),
            r(4.0, 8.0, 2.4, 0.0, 0.0, 5.0, 40.0), r(5.0, 9.5, 3.0, 0.0, 0.0, 6.0, 50.0), r(6.0, 12.0, 3.6, 0.0, 0.0, 8.0, 60.0), r(8.0, 16.0, 4.8, 0.0, 0.0, 10.0, 80.0),
            r(10.0, 20.0, 6.0, 0.0, 0.0, 12.0, 80.0),
        ],
    },
    ScrewStd {
        id: "gb68", code: "GB/T 68", zh: "开槽沉头螺钉", en: "Slotted countersunk screw", head: Head::Csk, drive: Drive::Slot,
        aliases: &["gb68", "GB/T 68", "开槽沉头", "一字沉头", "slotted countersunk"],
        rows: CSK_CROSS,
    },
    ScrewStd {
        id: "gb77", code: "GB/T 77", zh: "内六角平端紧定螺钉", en: "Socket set screw, flat point", head: Head::Set, drive: Drive::Socket,
        aliases: &["gb77", "GB/T 77", "平端紧定", "flat point"],
        rows: SET_ROWS,
    },
    ScrewStd {
        id: "gb78", code: "GB/T 78", zh: "内六角锥端紧定螺钉", en: "Socket set screw, cone point", head: Head::Set, drive: Drive::Socket,
        aliases: &["gb78", "GB/T 78", "锥端紧定", "cone point"],
        rows: SET_ROWS,
    },
    ScrewStd {
        id: "gb79", code: "GB/T 79", zh: "内六角圆柱端紧定螺钉", en: "Socket set screw, dog point", head: Head::Set, drive: Drive::Socket,
        aliases: &["gb79", "GB/T 79", "圆柱端紧定", "dog point"],
        rows: SET_ROWS,
    },
    ScrewStd {
        id: "gb80", code: "GB/T 80", zh: "内六角凹端紧定螺钉", en: "Socket set screw, cup point", head: Head::Set, drive: Drive::Socket,
        aliases: &["gb80", "GB/T 80", "凹端紧定", "cup point"],
        rows: SET_ROWS,
    },
];

/// 垫圈标准: (d, 内径 d1, 外径 d2, 厚 h)。弹垫 h = 压平后的厚度 s(算长度用)。
#[derive(Debug)]
pub struct WasherStd {
    pub id: &'static str,
    pub code: &'static str,
    pub zh: &'static str,
    pub en: &'static str,
    pub spring: bool,
    pub aliases: &'static [&'static str],
    pub rows: &'static [(f64, f64, f64, f64)],
}

pub const WASHERS: &[WasherStd] = &[
    WasherStd {
        id: "gb97.1", code: "GB/T 97.1", zh: "平垫圈", en: "Plain washer", spring: false, aliases: &["97.1", "平垫", "plain washer"],
        rows: &[
            (1.6, 1.7, 4.0, 0.3), (2.0, 2.2, 5.0, 0.3), (2.5, 2.7, 6.0, 0.5), (3.0, 3.2, 7.0, 0.5), (4.0, 4.3, 9.0, 0.8), (5.0, 5.3, 10.0, 1.0), (6.0, 6.4, 12.0, 1.6),
            (8.0, 8.4, 16.0, 1.6), (10.0, 10.5, 20.0, 2.0), (12.0, 13.0, 24.0, 2.5), (14.0, 15.0, 28.0, 2.5), (16.0, 17.0, 30.0, 3.0), (20.0, 21.0, 37.0, 3.0),
            (24.0, 25.0, 44.0, 4.0),
        ],
    },
    WasherStd {
        id: "gb848", code: "GB/T 848", zh: "小垫圈", en: "Small washer", spring: false, aliases: &["848", "小垫"],
        rows: &[
            (1.6, 1.7, 3.5, 0.3), (2.0, 2.2, 4.5, 0.3), (2.5, 2.7, 5.0, 0.5), (3.0, 3.2, 6.0, 0.5), (4.0, 4.3, 8.0, 0.5), (5.0, 5.3, 9.0, 1.0), (6.0, 6.4, 11.0, 1.6),
            (8.0, 8.4, 15.0, 1.6), (10.0, 10.5, 18.0, 1.6), (12.0, 13.0, 20.0, 2.0), (14.0, 15.0, 24.0, 2.5), (16.0, 17.0, 28.0, 2.5), (20.0, 21.0, 34.0, 3.0),
            (24.0, 25.0, 39.0, 4.0),
        ],
    },
    WasherStd {
        id: "gb96.1", code: "GB/T 96.1", zh: "大垫圈", en: "Large washer", spring: false, aliases: &["96.1", "大垫"],
        rows: &[
            (3.0, 3.2, 9.0, 0.8), (4.0, 4.3, 12.0, 1.0), (5.0, 5.3, 15.0, 1.0), (6.0, 6.4, 18.0, 1.6), (8.0, 8.4, 24.0, 2.0), (10.0, 10.5, 30.0, 2.5),
            (12.0, 13.0, 37.0, 3.0), (14.0, 15.0, 44.0, 3.0), (16.0, 17.0, 50.0, 3.0), (20.0, 22.0, 60.0, 4.0), (24.0, 26.0, 72.0, 5.0),
        ],
    },
    WasherStd {
        id: "gb93", code: "GB/T 93", zh: "标准型弹簧垫圈", en: "Spring lock washer", spring: true, aliases: &["gb93", "GB/T 93", "弹垫", "弹簧垫", "spring washer"],
        rows: &[
            (2.0, 2.1, 3.1, 0.5), (2.5, 2.6, 3.9, 0.65), (3.0, 3.1, 4.7, 0.8), (4.0, 4.1, 6.3, 1.1), (5.0, 5.1, 7.7, 1.3), (6.0, 6.1, 9.3, 1.6), (8.0, 8.1, 12.3, 2.1),
            (10.0, 10.2, 15.4, 2.6), (12.0, 12.2, 18.4, 3.1), (16.0, 16.2, 24.4, 4.1), (20.0, 20.2, 30.2, 5.0), (24.0, 24.5, 36.5, 6.0),
        ],
    },
];

/// 螺母标准: (d, 对边 s, 厚 m)。
#[derive(Debug)]
pub struct NutStd {
    pub id: &'static str,
    pub code: &'static str,
    pub zh: &'static str,
    pub en: &'static str,
    /// 尼龙锁紧(不和弹垫一起用)
    pub nylon: bool,
    pub aliases: &'static [&'static str],
    pub rows: &'static [(f64, f64, f64)],
}

pub const NUTS: &[NutStd] = &[
    NutStd {
        id: "gb6170", code: "GB/T 6170", zh: "1 型六角螺母", en: "Hex nut, style 1", nylon: false, aliases: &["6170", "六角螺母", "hex nut"],
        rows: &[
            (1.6, 3.2, 1.3), (2.0, 4.0, 1.6), (2.5, 5.0, 2.0), (3.0, 5.5, 2.4), (4.0, 7.0, 3.2), (5.0, 8.0, 4.7), (6.0, 10.0, 5.2), (8.0, 13.0, 6.8),
            (10.0, 16.0, 8.4), (12.0, 18.0, 10.8), (14.0, 21.0, 12.8), (16.0, 24.0, 14.8), (20.0, 30.0, 18.0), (24.0, 36.0, 21.5),
        ],
    },
    NutStd {
        id: "gb6172.1", code: "GB/T 6172.1", zh: "六角薄螺母", en: "Hex thin nut", nylon: false, aliases: &["6172", "薄螺母", "thin nut"],
        rows: &[
            (3.0, 5.5, 1.8), (4.0, 7.0, 2.2), (5.0, 8.0, 2.7), (6.0, 10.0, 3.2), (8.0, 13.0, 4.0), (10.0, 16.0, 5.0), (12.0, 18.0, 6.0), (14.0, 21.0, 7.0),
            (16.0, 24.0, 8.0), (20.0, 30.0, 10.0), (24.0, 36.0, 12.0),
        ],
    },
    NutStd {
        id: "gb889.1", code: "GB/T 889.1", zh: "尼龙锁紧螺母", en: "Nylon insert lock nut", nylon: true, aliases: &["889", "尼龙", "防松螺母", "nylon"],
        rows: &[
            (3.0, 5.5, 4.5), (4.0, 7.0, 6.0), (5.0, 8.0, 6.8), (6.0, 10.0, 8.0), (8.0, 13.0, 9.5), (10.0, 16.0, 11.9), (12.0, 18.0, 14.9), (14.0, 21.0, 17.0),
            (16.0, 24.0, 19.1), (20.0, 30.0, 22.8), (24.0, 36.0, 27.1),
        ],
    },
];

pub fn screw_std(id: &str) -> Option<&'static ScrewStd> {
    SCREWS.iter().find(|s| s.id == id)
}
pub fn washer_std(id: &str) -> Option<&'static WasherStd> {
    WASHERS.iter().find(|s| s.id == id)
}
pub fn nut_std(id: &str) -> Option<&'static NutStd> {
    NUTS.iter().find(|s| s.id == id)
}

fn same(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

impl ScrewStd {
    pub fn row(&self, d: f64) -> Option<Row> {
        self.rows.iter().copied().find(|r| same(r.d, d))
    }
    /// 该规格可选的公称长度。
    pub fn lengths(&self, d: f64) -> Vec<f64> {
        self.row(d).map(|r| L_SERIES.iter().copied().filter(|l| *l >= r.lmin - 1e-9 && *l <= r.lmax + 1e-9).collect()).unwrap_or_default()
    }
    /// 头露在支承面以上的高度(沉头 / 紧定 = 0, 头沉在孔里)。
    pub fn head_above(&self, d: f64) -> f64 {
        match self.head {
            Head::Csk | Head::Set => 0.0,
            _ => self.row(d).map_or(0.0, |r| r.k),
        }
    }
}
impl WasherStd {
    pub fn row(&self, d: f64) -> Option<(f64, f64, f64, f64)> {
        self.rows.iter().copied().find(|r| same(r.0, d))
    }
}
impl NutStd {
    pub fn row(&self, d: f64) -> Option<(f64, f64, f64)> {
        self.rows.iter().copied().find(|r| same(r.0, d))
    }
}

/// 一个标准件请求(给 MCP / 界面)。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PartReq {
    /// screw(配 std = 目录 id)/ screw-cap / screw-csk / screw-hex / screw-set / nut / washer / pin / shaft
    pub kind: String,
    /// 目录里的标准 id(screw: 必填, 如 "gb818"; washer: 默认 gb97.1, 可 gb848 / gb96.1 / gb93; nut: 默认 gb6170, 可 gb6172.1 / gb889.1)
    #[serde(default)]
    pub std: Option<String>,
    /// "M4" / "4"(螺纹件); 销 = 直径。
    #[serde(default)]
    pub size: Option<Value>,
    /// 螺钉 = 公称长度(沉头含头); 销 = 总长。
    #[serde(default)]
    pub length: Option<f64>,
    /// 轴: [[直径, 长度], …] 从 at 起沿 axis。
    #[serde(default)]
    pub steps: Option<Vec<[f64; 2]>>,
    /// 轴两端倒角(默认 0.5; 0 = 不倒)。
    #[serde(default)]
    pub chamfer: Option<f64>,
    #[serde(default = "origin")]
    pub at: [f64; 3],
    #[serde(default = "down")]
    pub axis: [f64; 3],
}

fn origin() -> [f64; 3] {
    [0.0, 0.0, 0.0]
}
fn down() -> [f64; 3] {
    [0.0, 0.0, -1.0]
}

fn r2(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn size_of(v: &Option<Value>) -> Result<f64, String> {
    let s = match v {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => s.trim().trim_start_matches(['M', 'm']).parse::<f64>().map_err(|_| format!("bad size {s:?} (like \"M4\")"))?,
        _ => return Err("size is required (like \"M4\")".into()),
    };
    if s > 0.0 {
        Ok(s)
    } else {
        Err("size must be > 0".into())
    }
}

fn have(ds: impl Iterator<Item = f64>) -> String {
    ds.map(|d| format!("M{d}")).collect::<Vec<_>>().join(" ")
}

fn screw_row(id: &str, d: f64) -> Result<(&'static ScrewStd, Row), String> {
    let s = screw_std(id).ok_or_else(|| format!("unknown screw standard {id} (have: {})", SCREWS.iter().map(|s| s.id).collect::<Vec<_>>().join(" ")))?;
    let r = s.row(d).ok_or_else(|| format!("{}: no M{d} in the table (have: {})", s.code, have(s.rows.iter().map(|r| r.d))))?;
    Ok((s, r))
}

/// 粗牙螺距(没有就按 0.15 d)。
pub fn pitch(d: f64) -> f64 {
    PITCH.iter().find(|p| (p.0 - d).abs() < 1e-9).map(|p| p.1).unwrap_or(0.15 * d)
}

fn norm(a: [f64; 3]) -> Result<[f64; 3], String> {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    if l > 0.0 {
        Ok([a[0] / l, a[1] / l, a[2] / l])
    } else {
        Err("axis must be non-zero".into())
    }
}

fn add(p: [f64; 3], a: [f64; 3], s: f64) -> [f64; 3] {
    [r2(p[0] + a[0] * s), r2(p[1] + a[1] * s), r2(p[2] + a[2] * s)]
}

/// 展开结果: 步骤(最后一步 id = "part")+ 规格 key(与 std_parts 识别同格式, 用来回验)+ 说明。
#[derive(Debug)]
pub struct Expanded {
    pub ops: Vec<Value>,
    pub spec: String,
    pub text: String,
}

fn fmt(x: f64) -> String {
    let s = format!("{x:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// 带头螺钉(目录里任一标准)→ 步骤。轴向坐标从杆端(z = 0)到头顶; 旋转轴朝头的方向。
fn screw_ops(st: &ScrewStd, row: Row, l: f64, at: [f64; 3], ax: [f64; 3]) -> Result<Expanded, String> {
    let neg = [-ax[0], -ax[1], -ax[2]];
    let Row { d, dk, k, s, t, .. } = row;
    let c = r2(0.6 * pitch(d));
    let shank = |top: f64| vec![[0.0, 0.0], [d / 2.0 - c, 0.0], [d / 2.0, c], [d / 2.0, top]];
    let (pts, socket_at): (Vec<[f64; 2]>, [f64; 3]) = match st.head {
        Head::Cap | Head::Cheese => {
            let hc = r2(0.08 * k);
            let mut p = shank(l);
            p.extend([[dk / 2.0, l], [dk / 2.0, r2(l + k - hc)], [r2(dk / 2.0 - hc), r2(l + k)], [0.0, r2(l + k)]]);
            (p, add(at, neg, k))
        }
        Head::Button | Head::Pan => {
            // 圆顶: 侧边直段到 0.2k(半圆头)/ 0.5k(盘头), 再按椭圆弧收到顶面 r = 0.3dk / 0.4dk
            let (side, top_r) = if st.head == Head::Button { (0.2, 0.3) } else { (0.5, 0.4) };
            let mut p = shank(l);
            p.push([dk / 2.0, l]);
            let z0 = l + side * k;
            let a_end = (top_r * 2.0f64).acos(); // cos θ = r / (dk/2)
            for i in 0..=4 {
                let th = a_end * i as f64 / 4.0;
                let z = z0 + (k - side * k) * th.sin() / a_end.sin();
                p.push([r2(dk / 2.0 * th.cos()), r2(z)]);
            }
            p.push([0.0, r2(l + k)]);
            (p, add(at, neg, k))
        }
        Head::Csk => {
            if l <= k + 1.0 {
                return Err(format!("length must be > head height {k:.2} + 1"));
            }
            let mut p = shank(r2(l - k));
            p.extend([[dk / 2.0, l], [0.0, l]]);
            (p, at)
        }
        Head::Hex | Head::Set => unreachable!(),
    };
    // 杆端 = at + axis × l(圆柱 / 盘头: at = 支承面; 沉头: at = 头顶, l 含头)
    let tip = add(at, ax, l);
    let profile: Vec<Value> = pts.iter().map(|q| json!([q[0], q[1]])).collect();
    let mut ops = vec![json!({"op": "revolve", "id": "body", "origin": tip, "axis": neg, "profile": profile})];
    if st.drive == Drive::Socket && s > 0.0 {
        ops.push(json!({"op": "prism", "id": "part", "center": socket_at, "normal": ax, "sides": 6, "flats": s, "height": t, "bool": "subtract", "targets": ["$body"]}));
    } else {
        ops[0]["id"] = json!("part");
    }
    let key = match st.head {
        Head::Cap => "screw-cap",
        Head::Csk => "screw-csk",
        _ => "screw",
    };
    Ok(Expanded {
        ops,
        spec: format!("{key} M{}x{}", fmt(d), fmt(l)),
        text: format!(
            "{} M{}×{} {}(dk {dk}, k {k}{}){}",
            st.code,
            fmt(d),
            fmt(l),
            st.zh,
            if s > 0.0 { format!(", s {s}, t {t}") } else { String::new() },
            if matches!(st.drive, Drive::Cross | Drive::Slot) { " — 槽未建" } else { "" }
        ),
    })
}

/// 请求 → 建模步骤。
pub fn expand(q: &PartReq) -> Result<Expanded, String> {
    let ax = norm(q.axis)?;
    let neg = [-ax[0], -ax[1], -ax[2]];
    let at = q.at;
    let len = || q.length.filter(|l| *l > 0.0).ok_or_else(|| "length is required".to_string());
    // 旧 kind → 目录里的标准
    let kind = q.kind.as_str();
    let std_for_kind = match kind {
        "screw-cap" => Some("gb70.1"),
        "screw-csk" => Some("gb70.3"),
        "screw-hex" => Some("gb5783"),
        "screw-set" => Some("gb77"),
        _ => None,
    };
    let screw_id = match (kind, std_for_kind) {
        ("screw", _) => Some(q.std.clone().ok_or("screw needs std (a catalog id like gb70.1 / gb818)")?),
        (_, Some(s)) => Some(s.to_string()),
        _ => None,
    };
    if let Some(id) = screw_id {
        let d = size_of(&q.size)?;
        let (st, row) = screw_row(&id, d)?;
        let l = len()?;
        let Row { dk, k, s, t, .. } = row;
        return match st.head {
            Head::Hex => {
                let c = r2(0.6 * pitch(d));
                let tip = add(at, ax, l);
                let profile = json!([[0, 0], [d / 2.0 - c, 0], [d / 2.0, c], [d / 2.0, l], [0, l]]);
                Ok(Expanded {
                    ops: vec![
                        json!({"op": "revolve", "id": "body", "origin": tip, "axis": neg, "profile": profile}),
                        json!({"op": "prism", "id": "part", "center": at, "normal": neg, "sides": 6, "flats": dk, "height": k, "bool": "union", "targets": ["$body"]}),
                    ],
                    spec: format!("screw-hex M{}x{}", fmt(d), fmt(l)),
                    text: format!("{} M{}×{} {}(s {dk}, k {k})", st.code, fmt(d), fmt(l), st.zh),
                })
            }
            Head::Set => {
                let c = r2(0.6 * pitch(d));
                let far = add(at, ax, l);
                let profile = json!([[0, 0], [d / 2.0 - c, 0], [d / 2.0, c], [d / 2.0, r2(l - c)], [d / 2.0 - c, l], [0, l]]);
                Ok(Expanded {
                    ops: vec![
                        json!({"op": "revolve", "id": "body", "origin": far, "axis": neg, "profile": profile}),
                        json!({"op": "prism", "id": "part", "center": at, "normal": ax, "sides": 6, "flats": s, "height": t, "bool": "subtract", "targets": ["$body"]}),
                    ],
                    spec: format!("screw-set M{}x{}", fmt(d), fmt(l)),
                    text: format!("{} M{}×{} {}(s {s}, t {t}){}", st.code, fmt(d), fmt(l), st.zh, if st.id == "gb77" { "" } else { " — 端部按平端建" }),
                })
            }
            _ => screw_ops(st, row, l, at, ax),
        };
    }
    match kind {
        "nut" => {
            let d = size_of(&q.size)?;
            let ns = nut_std(q.std.as_deref().unwrap_or("gb6170")).ok_or("unknown nut standard (gb6170 / gb6172.1 / gb889.1)")?;
            let (_, s, m) = ns.row(d).ok_or_else(|| format!("{}: no M{d} in the table (have: {})", ns.code, have(ns.rows.iter().map(|r| r.0))))?;
            Ok(Expanded {
                ops: vec![
                    json!({"op": "prism", "id": "body", "center": at, "normal": ax, "sides": 6, "flats": s, "height": m}),
                    json!({"op": "cylinder", "id": "part", "center": add(at, neg, 0.5), "axis": ax, "radius": d / 2.0, "height": m + 1.0, "bool": "subtract", "targets": ["$body"]}),
                ],
                spec: format!("nut M{} m{}", fmt(d), fmt(m)),
                text: format!("{} M{} {}(s {s}, m {m})", ns.code, fmt(d), ns.zh),
            })
        }
        "washer" => {
            let d = size_of(&q.size)?;
            let ws = washer_std(q.std.as_deref().unwrap_or("gb97.1")).ok_or("unknown washer standard (gb97.1 / gb848 / gb96.1 / gb93)")?;
            let (_, d1, d2, h) = ws.row(d).ok_or_else(|| format!("{}: no M{d} in the table (have: {})", ws.code, have(ws.rows.iter().map(|r| r.0))))?;
            let profile = json!([[d1 / 2.0, 0], [d2 / 2.0, 0], [d2 / 2.0, h], [d1 / 2.0, h]]);
            Ok(Expanded {
                ops: vec![json!({"op": "revolve", "id": "part", "origin": at, "axis": ax, "profile": profile})],
                spec: format!("washer M{} OD{} t{}", fmt(d), fmt(d2), fmt(h)),
                text: format!("{} {} {}(d1 {d1}, d2 {d2}, h {h}){}", ws.code, fmt(d), ws.zh, if ws.spring { " — 按压平建, 开口未建" } else { "" }),
            })
        }
        "pin" => {
            let d = size_of(&q.size)?;
            let l = len()?;
            let c = r2(q.chamfer.unwrap_or((0.1 * d).max(0.2)).min(d / 4.0));
            let profile = if c > 0.0 {
                json!([[0, 0], [d / 2.0 - c, 0], [d / 2.0, c], [d / 2.0, r2(l - c)], [d / 2.0 - c, l], [0, l]])
            } else {
                json!([[0, 0], [d / 2.0, 0], [d / 2.0, l], [0, l]])
            };
            Ok(Expanded {
                ops: vec![json!({"op": "revolve", "id": "part", "origin": at, "axis": ax, "profile": profile})],
                spec: format!("pin D{} L{}", fmt(d), fmt(l)),
                text: format!("GB/T 119.2 ⌀{}×{} 圆柱销", fmt(d), fmt(l)),
            })
        }
        "shaft" => {
            let steps = q.steps.clone().filter(|s| !s.is_empty()).ok_or("shaft needs steps [[diameter, length], …]")?;
            if steps.iter().any(|s| !(s[0] > 0.0 && s[1] > 0.0)) {
                return Err("every step needs diameter > 0 and length > 0".into());
            }
            let c = q.chamfer.unwrap_or(0.5).max(0.0);
            let total: f64 = steps.iter().map(|s| s[1]).sum();
            // 轮廓: 沿 z 逐段, 台阶处直角; 两端倒角 c(不超过端部那段的半径 / 长度的一半)
            let mut pts: Vec<[f64; 2]> = vec![[0.0, 0.0]];
            let (d0, l0) = (steps[0][0], steps[0][1]);
            let c0 = c.min(d0 / 4.0).min(l0 / 2.0);
            if c0 > 0.0 {
                pts.push([d0 / 2.0 - c0, 0.0]);
                pts.push([d0 / 2.0, c0]);
            } else {
                pts.push([d0 / 2.0, 0.0]);
            }
            let mut z = 0.0;
            for (i, s) in steps.iter().enumerate() {
                if i > 0 {
                    pts.push([s[0] / 2.0, r2(z)]);
                }
                z += s[1];
                if i + 1 < steps.len() {
                    pts.push([s[0] / 2.0, r2(z)]);
                }
            }
            let (dn, ln) = (steps[steps.len() - 1][0], steps[steps.len() - 1][1]);
            let cn = c.min(dn / 4.0).min(ln / 2.0);
            if cn > 0.0 {
                pts.push([dn / 2.0, r2(total - cn)]);
                pts.push([dn / 2.0 - cn, r2(total)]);
            } else {
                pts.push([dn / 2.0, r2(total)]);
            }
            pts.push([0.0, r2(total)]);
            Ok(Expanded {
                ops: vec![json!({"op": "revolve", "id": "part", "origin": at, "axis": ax, "profile": pts})],
                spec: String::new(),
                text: format!("阶梯轴 {} 段, 总长 {}", steps.len(), fmt(total)),
            })
        }
        k => Err(format!("unknown kind {k} (screw / screw-cap / screw-csk / screw-hex / screw-set / nut / washer / pin / shaft)")),
    }
}

/// 支持的规格(给工具说明)。
pub fn sizes() -> String {
    let mut out: Vec<String> = SCREWS.iter().map(|s| format!("{} {} ({}): {}", s.id, s.code, s.en, have(s.rows.iter().map(|r| r.d)))).collect();
    out.extend(WASHERS.iter().map(|s| format!("washer {} {} ({}): {}", s.id, s.code, s.en, have(s.rows.iter().map(|r| r.0)))));
    out.extend(NUTS.iter().map(|s| format!("nut {} {} ({}): {}", s.id, s.code, s.en, have(s.rows.iter().map(|r| r.0)))));
    out.push("pin (GB/T 119.2): any diameter; shaft: any steps".into());
    out.join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(v: Value) -> PartReq {
        serde_json::from_value(v).unwrap()
    }

    /// 轮廓 r 都 ≥ 0、z 单调不减到顶、首尾在轴上(垫圈除外)
    fn profile_ok(p: &Value) -> Vec<[f64; 2]> {
        let pts: Vec<[f64; 2]> = serde_json::from_value(p.clone()).unwrap();
        assert!(pts.iter().all(|q| q[0] >= 0.0), "{pts:?}");
        pts
    }

    #[test]
    fn cap_screw_m4x12() {
        let e = expand(&req(json!({"kind": "screw-cap", "size": "M4", "length": 12, "at": [300, 0, 16], "axis": [0, 0, -1]}))).unwrap();
        assert_eq!(e.spec, "screw-cap M4x12");
        let rev = &e.ops[0];
        // 杆端在 at + axis × 12 = (300, 0, 4); 旋转轴朝头(+Z)
        assert_eq!(rev["origin"], json!([300.0, 0.0, 4.0]));
        assert_eq!(rev["axis"], json!([-0.0, -0.0, 1.0]));
        let pts = profile_ok(&rev["profile"]);
        assert_eq!(pts[3], [2.0, 12.0], "杆到支承面");
        assert_eq!(pts[4], [3.5, 12.0], "头径 7");
        assert_eq!(pts.last().unwrap()[1], 16.0, "头高 4");
        assert_eq!(pts[1], [1.58, 0.0], "端部倒角 0.6 × 0.7");
        let soc = &e.ops[1];
        // 内六角从头顶(at − axis × k = (300, 0, 20))往杆的方向挖 t = 2
        assert_eq!(soc["center"], json!([300.0, 0.0, 20.0]));
        assert_eq!((soc["flats"].as_f64(), soc["height"].as_f64(), soc["bool"].as_str()), (Some(3.0), Some(2.0), Some("subtract")));
        // 新写法(目录 id)与旧 kind 一致
        let e2 = expand(&req(json!({"kind": "screw", "std": "gb70.1", "size": "M4", "length": 12, "at": [300, 0, 16], "axis": [0, 0, -1]}))).unwrap();
        assert_eq!(e2.ops, e.ops);
    }

    #[test]
    fn other_kinds_and_errors() {
        let csk = expand(&req(json!({"kind": "screw-csk", "size": 4, "length": 12}))).unwrap();
        let p = profile_ok(&csk.ops[0]["profile"]);
        assert_eq!(p[3], [2.0, 9.52], "沉头锥起点 = l − (dk − d)/2");
        assert_eq!(p[4], [4.48, 12.0]);
        let hex = expand(&req(json!({"kind": "screw-hex", "size": "M8", "length": 30}))).unwrap();
        assert_eq!((hex.ops[1]["flats"].as_f64(), hex.ops[1]["height"].as_f64(), hex.ops[1]["bool"].as_str()), (Some(13.0), Some(5.3), Some("union")));
        let nut = expand(&req(json!({"kind": "nut", "size": "M6"}))).unwrap();
        assert_eq!(nut.spec, "nut M6 m5.2");
        let w = expand(&req(json!({"kind": "washer", "size": "M4"}))).unwrap();
        assert_eq!(w.spec, "washer M4 OD9 t0.8");
        assert_eq!(profile_ok(&w.ops[0]["profile"])[0], [2.15, 0.0], "垫圈轮廓不碰轴");
        let pin = expand(&req(json!({"kind": "pin", "size": 5, "length": 20}))).unwrap();
        assert_eq!(pin.spec, "pin D5 L20");
        let sh = expand(&req(json!({"kind": "shaft", "steps": [[10, 20], [15, 30], [10, 15]], "chamfer": 1}))).unwrap();
        let p = profile_ok(&sh.ops[0]["profile"]);
        assert_eq!(p.first().unwrap(), &[0.0, 0.0]);
        assert_eq!(p.last().unwrap(), &[0.0, 65.0]);
        assert!(p.contains(&[7.5, 20.0]) && p.contains(&[7.5, 50.0]) && p.contains(&[5.0, 50.0]), "{p:?}");
        assert!(expand(&req(json!({"kind": "screw-cap", "size": "M7", "length": 10}))).unwrap_err().contains("no M7"));
        assert!(expand(&req(json!({"kind": "screw-cap", "size": "M4"}))).unwrap_err().contains("length"));
        assert!(expand(&req(json!({"kind": "rivet", "size": "M4"}))).is_err());
        assert!(expand(&req(json!({"kind": "screw", "size": "M4", "length": 10}))).unwrap_err().contains("std"));
        assert!(sizes().contains("GB/T 70.1") && sizes().contains("GB/T 889.1"));
    }

    #[test]
    fn catalog_heads_and_accessories() {
        // 所有标准的所有规格都能建(取最短 / 最长的一个长度)
        for st in SCREWS {
            for row in st.rows {
                let ls = st.lengths(row.d);
                assert!(!ls.is_empty(), "{} M{} has no lengths", st.code, row.d);
                for l in [ls[0], *ls.last().unwrap()] {
                    if st.head == Head::Csk && l <= row.k + 1.0 {
                        continue;
                    }
                    let e = expand(&req(json!({"kind": "screw", "std": st.id, "size": row.d, "length": l})));
                    let e = e.unwrap_or_else(|e| panic!("{} M{}x{l}: {e}", st.code, row.d));
                    assert_eq!(e.ops.last().unwrap()["id"], "part");
                    let pts = profile_ok(&e.ops[0]["profile"]);
                    let ztop = pts.iter().map(|q| q[1]).fold(0.0, f64::max);
                    let want = if matches!(st.head, Head::Csk | Head::Set | Head::Hex) { l } else { l + row.k };
                    assert!((ztop - want).abs() < 1e-6, "{} M{}x{l}: top {ztop} want {want}", st.code, row.d);
                }
            }
        }
        assert_eq!(screw_std("gb70.2").unwrap().lengths(4.0), vec![6.0, 8.0, 10.0, 12.0, 16.0, 20.0]);
        assert_eq!(screw_std("gb70.3").unwrap().head_above(4.0), 0.0);
        assert_eq!(screw_std("gb818").unwrap().head_above(4.0), 3.1);
        let sp = expand(&req(json!({"kind": "washer", "std": "gb93", "size": 6}))).unwrap();
        assert_eq!(sp.spec, "washer M6 OD9.3 t1.6");
        let ny = expand(&req(json!({"kind": "nut", "std": "gb889.1", "size": 8}))).unwrap();
        assert_eq!(ny.spec, "nut M8 m9.5");
        assert!(expand(&req(json!({"kind": "nut", "std": "gb889.1", "size": 2}))).unwrap_err().contains("no M2"));
    }

    /// 宏建出来的形状应被 std_parts 认成同一规格(按真机 bom.scan 的形状摘要合成)
    #[test]
    fn recognizer_agrees_with_macros() {
        let cyl = |r: f64, h: f64, inner: bool| json!({"t": "cyl", "r": r, "h": h, "inner": inner, "axis": [0, 0, 1], "o": [0, 0, 0]});
        let cone = |r: f64, inner: bool| json!({"t": "cone", "r": r, "inner": inner, "axis": [0, 0, 1], "o": [0, 0, 0]});
        // M4×12 杯头: 杆 r2 h(12 − 0.42), 头 r3.5 h(4 − 0.32), 6 个内六角侧面 + 孔底 + 头顶 + 支承面 + 杆端 = 10 平面, 两个外锥
        let s = crate::std_parts::classify(&json!({"len": 16.0, "planes": 10, "rev": [cyl(2.0, 11.58, false), cyl(3.5, 3.68, false), cone(1.79, false), cone(3.34, false)]})).unwrap();
        assert_eq!(s.key, expand(&req(json!({"kind": "screw-cap", "size": "M4", "length": 12}))).unwrap().spec);
        let n = crate::std_parts::classify(&json!({"len": 5.2, "planes": 8, "rev": [cyl(3.0, 5.2, true)]})).unwrap();
        assert_eq!(n.key, expand(&req(json!({"kind": "nut", "size": "M6"}))).unwrap().spec);
        let w = crate::std_parts::classify(&json!({"len": 0.8, "planes": 2, "rev": [cyl(2.15, 0.8, true), cyl(4.5, 0.8, false)]})).unwrap();
        assert_eq!(w.key, expand(&req(json!({"kind": "washer", "size": "M4"}))).unwrap().spec);
    }
}
