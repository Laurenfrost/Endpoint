//! 规则库:章节 / 卷 / 水印识别共享的基础设施。
//!
//! CLAUDE.md 第四节明确「规则库被章节解析与水印检测共享,因此独立成模块」。本模块只负责
//! 规则的**存储与组织**(类型、内置默认集、JSON I/O、按优先级排序),不掺杂任何业务逻辑——
//! 具体如何用规则(扫描章节标题 / 检测水印行)由消费者决定。
//!
//! # 规则 ID 命名约定
//! - 内置规则:`builtin-<kind>-<slug>`(如 `builtin-chapter-cn-zhang`)
//! - 用户规则:由 UI 生成 UUID 或允许用户自定义 ID
//! - LLM 规则:`llm-<kind>-<short-hash>`
//!
//! # 优先级
//! 数字越大优先级越高,扫描器先尝试高优先级的规则。当同一行能被多条规则同时命中时,
//! 第一个匹配赢——所以更具体的模式应当给更高的优先级。

use std::fs;
use std::io;
use std::path::Path;

use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, warn};

#[derive(Debug, Error)]
pub enum RulesError {
    #[error("无法读取规则文件 {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("无法写入规则文件 {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("规则文件 {path} JSON 解析失败: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("规则 `{id}` 的正则编译失败: {source}")]
    BadRegex {
        id: String,
        #[source]
        source: regex::Error,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    Chapter,
    Volume,
    /// 水印检测规则。阶段一不消费,但写进类型集以避免阶段三再改类型。
    Watermark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleSource {
    Builtin,
    User,
    LlmGenerated,
}

/// 单条规则。`pattern` 是 Rust `regex` crate 兼容的正则,通常匹配整行
/// (内含 `^...$`)。但具体语义由消费者解释。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub pattern: String,
    pub kind: RuleKind,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub priority: i32,
    pub source: RuleSource,
    #[serde(default)]
    pub description: String,
}

fn default_enabled() -> bool {
    true
}

impl Rule {
    pub fn builtin(
        id: &str,
        pattern: &str,
        kind: RuleKind,
        priority: i32,
        description: &str,
    ) -> Self {
        Self {
            id: id.to_string(),
            pattern: pattern.to_string(),
            kind,
            enabled: true,
            priority,
            source: RuleSource::Builtin,
            description: description.to_string(),
        }
    }

    /// 编译 `pattern`。失败附带 rule id 以便排错。
    pub fn compile(&self) -> Result<Regex, RulesError> {
        Regex::new(&self.pattern).map_err(|e| {
            warn!(rule_id = %self.id, pattern = %self.pattern, error = %e, "规则正则编译失败");
            RulesError::BadRegex {
                id: self.id.clone(),
                source: e,
            }
        })
    }
}

/// 规则集:维护一组规则,提供按 kind 过滤、按优先级排序、JSON I/O。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuleSet {
    #[serde(default)]
    pub rules: Vec<Rule>,
}

impl RuleSet {
    /// 内置默认规则集。覆盖中文网文常见的章/卷标题。
    pub fn builtin() -> Self {
        Self {
            rules: builtin_rules(),
        }
    }

    /// 按 kind 过滤启用规则,按优先级降序排列。
    pub fn enabled_by_kind(&self, kind: RuleKind) -> Vec<&Rule> {
        let mut v: Vec<&Rule> = self
            .rules
            .iter()
            .filter(|r| r.enabled && r.kind == kind)
            .collect();
        v.sort_by(|a, b| b.priority.cmp(&a.priority));
        v
    }

    /// 按 id 查找规则(`O(n)`,规则数不会很大)。
    pub fn find(&self, id: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.id == id)
    }

    /// 追加或替换规则(按 id)。
    pub fn upsert(&mut self, rule: Rule) {
        if let Some(pos) = self.rules.iter().position(|r| r.id == rule.id) {
            self.rules[pos] = rule;
        } else {
            self.rules.push(rule);
        }
    }

    /// 按 id 删除规则。返回是否实际删除。
    pub fn remove(&mut self, id: &str) -> bool {
        let len = self.rules.len();
        self.rules.retain(|r| r.id != id);
        self.rules.len() != len
    }

    /// 合并:把 `other` 中的规则按 id 追加/覆盖到 self。常用法:用户规则覆盖内置同 id 规则。
    pub fn merge(&mut self, other: RuleSet) {
        let before = self.rules.len();
        let incoming = other.rules.len();
        for r in other.rules {
            self.upsert(r);
        }
        debug!(
            before,
            incoming,
            after = self.rules.len(),
            "规则集合并"
        );
    }

    pub fn load_from_json(path: &Path) -> Result<Self, RulesError> {
        let s = fs::read_to_string(path).map_err(|e| RulesError::Read {
            path: path.display().to_string(),
            source: e,
        })?;
        let set: Self = serde_json::from_str(&s).map_err(|e| RulesError::Parse {
            path: path.display().to_string(),
            source: e,
        })?;
        debug!(path = %path.display(), rules = set.rules.len(), "从 JSON 加载规则集");
        Ok(set)
    }

    pub fn save_to_json(&self, path: &Path) -> Result<(), RulesError> {
        let s = serde_json::to_string_pretty(self).map_err(|e| RulesError::Parse {
            path: path.display().to_string(),
            source: e,
        })?;
        fs::write(path, s).map_err(|e| RulesError::Write {
            path: path.display().to_string(),
            source: e,
        })
    }
}

// —— 内置规则集 ——
//
// 每条 pattern 都设计为「针对单行 trim 后的内容」匹配,带 `^...$` 锚点。
// 数字段允许中文数字、阿拉伯数字、零号(〇)、繁简「两」。
//
// 标题文本(章名)可选,前接空白或常见标点(冒号、破折号、顿号等)。

fn cn_or_arabic_digits() -> &'static str {
    // 中文数字 + 阿拉伯数字 + 〇 + 两(繁简)
    r"[0-9零一二三四五六七八九十百千万亿两〇壹贰叁肆伍陆柒捌玖拾佰仟]{1,15}"
}

fn builtin_rules() -> Vec<Rule> {
    let d = cn_or_arabic_digits();
    // 章/卷后跟可选标题:用空白或常见分隔符引出
    let title_tail = r"(?:[\s\.\-—–:：、].{0,60})?";
    vec![
        // —— 章节规则 ——
        Rule::builtin(
            "builtin-chapter-cn-zhang",
            &format!(r"^第{d}章{title_tail}$"),
            RuleKind::Chapter,
            200,
            "「第X章」格式章节标题(支持中文数字与阿拉伯数字)",
        ),
        Rule::builtin(
            "builtin-chapter-cn-hui",
            &format!(r"^第{d}回{title_tail}$"),
            RuleKind::Chapter,
            190,
            "「第X回」格式(古典/武侠风格)",
        ),
        Rule::builtin(
            "builtin-chapter-cn-hua",
            &format!(r"^第{d}话{title_tail}$"),
            RuleKind::Chapter,
            180,
            "「第X话」格式(轻小说风格)",
        ),
        Rule::builtin(
            "builtin-chapter-cn-jie",
            &format!(r"^第{d}[节節]{title_tail}$"),
            RuleKind::Chapter,
            170,
            "「第X节/節」格式",
        ),
        Rule::builtin(
            "builtin-chapter-prologue",
            r"^(?:序章|序言|楔子|前言|引子)(?:[\s\.\-—–:：、].{0,60})?$",
            RuleKind::Chapter,
            160,
            "楔子/序章/序言/前言/引子",
        ),
        Rule::builtin(
            "builtin-chapter-extra",
            r"^(?:番外|外传|后记|尾声|终章)(?:篇|章)?(?:[\s\.\-—–:：、].{0,60})?$",
            RuleKind::Chapter,
            150,
            "番外/外传/后记/尾声/终章",
        ),
        // —— 卷规则 ——
        Rule::builtin(
            "builtin-volume-cn-juan",
            &format!(r"^第{d}卷{title_tail}$"),
            RuleKind::Volume,
            200,
            "「第X卷」格式",
        ),
        Rule::builtin(
            "builtin-volume-juan-x",
            &format!(r"^卷{d}{title_tail}$"),
            RuleKind::Volume,
            190,
            "「卷X」格式",
        ),
        Rule::builtin(
            "builtin-volume-positional",
            r"^(?:上|中|下)卷(?:[\s\.\-—–:：、].{0,60})?$",
            RuleKind::Volume,
            180,
            "上卷/中卷/下卷",
        ),
        Rule::builtin(
            "builtin-volume-cn-bu",
            &format!(r"^第{d}部{title_tail}$"),
            RuleKind::Volume,
            200,
            "「第X部」格式",
        ),
        // —— 阶段三:水印规则 ——
        //
        // 这些规则匹配「整行任意位置」(不锚定 ^...$),用于 watermark 检测的关键词特征。
        // 命中即给该行加一条 `WatermarkSignal { kind: keyword_regex, score: 1.0 }`;
        // 单 keyword 命中默认权重融合 = 0.40,落 suspect 灰区(不会自动删除)。
        //
        // 收录原则:可解释 + 不易误命中正文 + 网文常见。模糊或激进的模式留待 LLM 规则生成器
        // (阶段四)产出,不作为内置。
        Rule::builtin(
            "builtin-watermark-url-http",
            r"https?://[A-Za-z0-9_./\-?#=&%+:~]+",
            RuleKind::Watermark,
            200,
            "HTTP/HTTPS URL",
        ),
        Rule::builtin(
            "builtin-watermark-url-www",
            r"(?:^|[\s\u{3000}(\[【「『])www\.[A-Za-z0-9_./\-]{3,}",
            RuleKind::Watermark,
            190,
            "www. 开头的网址简写",
        ),
        Rule::builtin(
            "builtin-watermark-tg-handle",
            // 至少 5 个字符,大幅降低普通正文(如 emoji 间隔的 @人名)误命中
            r"@[A-Za-z0-9_]{5,}",
            RuleKind::Watermark,
            150,
            "@开头的 Telegram / 社交账号",
        ),
        Rule::builtin(
            "builtin-watermark-domain",
            // 常见网文站常用 TLD;\b 避免在中文里乱触发
            r"\b[A-Za-z][A-Za-z0-9\-]{1,}\.(?:com|net|org|cc|cn|io|me|xyz|info|tv|club|app|top|vip|biz)\b",
            RuleKind::Watermark,
            180,
            "常见 TLD 域名(.com/.cc/.cn 等)",
        ),
        Rule::builtin(
            "builtin-watermark-first-publish",
            // 「本书/本文/本章 首发」、「首发于」、「更新最快」
            r"(?:本(?:文|书|章|站)\s*首发|首发(?:于|地址|平台|网址|站点)|更新最快|最快(?:更新|阅读))",
            RuleKind::Watermark,
            170,
            "「首发于」「更新最快」等推广水印",
        ),
        Rule::builtin(
            "builtin-watermark-piracy-warn",
            // 「盗版必究 / 抄袭可耻 / 严禁转载 / 搬运可耻」之类的版权声明
            r"(?:盗版必究|盗文可耻|抄袭可耻|搬运可耻|严禁转载|未经授权请勿转载)",
            RuleKind::Watermark,
            160,
            "版权声明 / 反盗版水印",
        ),
        Rule::builtin(
            "builtin-watermark-free-read",
            // 「免费阅读 / 无广告阅读 / 全文阅读」之类的推广行
            r"(?:免费(?:阅读|看书|小说)|无广告(?:阅读|追书)|全本(?:免费|无弹窗))",
            RuleKind::Watermark,
            140,
            "「免费阅读」「无广告」等推广水印",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches_any(rules: &[&Rule], line: &str) -> Option<String> {
        for r in rules {
            let re = r.compile().unwrap();
            if re.is_match(line) {
                return Some(r.id.clone());
            }
        }
        None
    }

    #[test]
    fn builtin_rules_all_compile() {
        let set = RuleSet::builtin();
        for r in &set.rules {
            r.compile().unwrap_or_else(|e| panic!("规则 {} 编译失败: {e}", r.id));
        }
    }

    #[test]
    fn chapter_rules_match_common_formats() {
        let set = RuleSet::builtin();
        let rules = set.enabled_by_kind(RuleKind::Chapter);
        for line in [
            "第一章 起",
            "第123章",
            "第二十三章 风起",
            "第一回 楔子",
            "第三话 决战",
            "第一节 序",
            "楔子",
            "序章 神之降临",
            "番外 平行时空",
            "番外篇 番外故事",
        ] {
            assert!(
                matches_any(&rules, line).is_some(),
                "未识别章节标题: {}",
                line
            );
        }
    }

    #[test]
    fn volume_rules_match_common_formats() {
        let set = RuleSet::builtin();
        let rules = set.enabled_by_kind(RuleKind::Volume);
        for line in ["第一卷 风起", "第一卷", "卷一 风起云涌", "上卷", "下卷"] {
            assert!(
                matches_any(&rules, line).is_some(),
                "未识别卷标题: {}",
                line
            );
        }
    }

    #[test]
    fn chapter_rules_reject_non_headings() {
        let set = RuleSet::builtin();
        let rules = set.enabled_by_kind(RuleKind::Chapter);
        for line in [
            "他翻开了第一章",
            "第一章节内容里出现的话",
            "随便一行正文",
            "「第一章 起」是这本书的开头",
            "",
        ] {
            assert!(
                matches_any(&rules, line).is_none(),
                "误识别为章节: {}",
                line
            );
        }
    }

    #[test]
    fn enabled_by_kind_sorts_by_priority_desc() {
        let set = RuleSet::builtin();
        let chs = set.enabled_by_kind(RuleKind::Chapter);
        for w in chs.windows(2) {
            assert!(
                w[0].priority >= w[1].priority,
                "未按优先级降序: {} ({}) before {} ({})",
                w[0].id,
                w[0].priority,
                w[1].id,
                w[1].priority
            );
        }
    }

    #[test]
    fn upsert_and_remove_work() {
        let mut set = RuleSet::builtin();
        let user_rule = Rule {
            id: "user-chapter-custom".into(),
            pattern: r"^自定义章$".into(),
            kind: RuleKind::Chapter,
            enabled: true,
            priority: 500,
            source: RuleSource::User,
            description: "用户自定义".into(),
        };
        set.upsert(user_rule.clone());
        assert!(set.find("user-chapter-custom").is_some());
        // upsert 同 id 时替换
        let mut replaced = user_rule.clone();
        replaced.priority = 999;
        set.upsert(replaced);
        assert_eq!(set.find("user-chapter-custom").unwrap().priority, 999);
        assert!(set.remove("user-chapter-custom"));
        assert!(set.find("user-chapter-custom").is_none());
    }

    #[test]
    fn disabled_rules_are_filtered() {
        let mut set = RuleSet::builtin();
        let id = "builtin-chapter-cn-zhang";
        let mut r = set.find(id).cloned().unwrap();
        r.enabled = false;
        set.upsert(r);
        let chs = set.enabled_by_kind(RuleKind::Chapter);
        assert!(chs.iter().all(|r| r.id != id));
    }

    #[test]
    fn json_round_trip_preserves_rules() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rules.json");
        let set = RuleSet::builtin();
        set.save_to_json(&path).unwrap();
        let loaded = RuleSet::load_from_json(&path).unwrap();
        assert_eq!(loaded.rules.len(), set.rules.len());
        // 抽查首条规则
        assert_eq!(loaded.rules[0].id, set.rules[0].id);
        assert_eq!(loaded.rules[0].pattern, set.rules[0].pattern);
    }

    fn matches_any_substring(rules: &[&Rule], line: &str) -> Option<String> {
        for r in rules {
            let re = r.compile().unwrap();
            if re.is_match(line) {
                return Some(r.id.clone());
            }
        }
        None
    }

    #[test]
    fn watermark_rules_all_compile() {
        let set = RuleSet::builtin();
        let wms: Vec<&Rule> = set
            .rules
            .iter()
            .filter(|r| r.kind == RuleKind::Watermark)
            .collect();
        assert!(wms.len() >= 5, "阶段三应当至少有 5 条内置 watermark 规则");
        for r in &wms {
            r.compile().unwrap_or_else(|e| panic!("watermark 规则 {} 编译失败: {e}", r.id));
        }
    }

    #[test]
    fn watermark_rules_match_typical_lines() {
        let set = RuleSet::builtin();
        let rules = set.enabled_by_kind(RuleKind::Watermark);
        // 每个 case 是一行典型的水印文本,应当至少有一条规则命中
        let positives = [
            "更多精彩内容请访问 https://novel.example.com/book/12345",
            "请到 www.somesite.cc 阅读最新章节",
            "关注 TG 频道 @somenovelchannel",
            "首发于 xyznovel.com,转载请注明",
            "本文首发于纵横中文网",
            "更新最快的小说网站",
            "盗版必究!",
            "未经授权请勿转载",
            "免费阅读全本小说",
            "无广告追书,体验更佳",
        ];
        for line in positives {
            assert!(
                matches_any_substring(&rules, line).is_some(),
                "未识别水印行: {}",
                line
            );
        }
    }

    #[test]
    fn watermark_rules_reject_normal_prose() {
        let set = RuleSet::builtin();
        let rules = set.enabled_by_kind(RuleKind::Watermark);
        // 这些是典型正文,**不**应被任何 watermark 规则命中
        let negatives = [
            "他抬起头,望着远方的天空。",
            "「你怎么了?」她轻声问道。",
            "夜色渐浓,街上的人也少了。",
            "第一章 起",  // 章节标题不应被 watermark 规则命中
            "第二卷 风云起",
            "嗯。",
            "「……」",
        ];
        for line in negatives {
            assert!(
                matches_any_substring(&rules, line).is_none(),
                "误识别为水印: {}",
                line
            );
        }
    }

    #[test]
    fn merge_overrides_by_id() {
        let mut a = RuleSet::builtin();
        let mut b = RuleSet::default();
        b.rules.push(Rule {
            id: "builtin-chapter-cn-zhang".into(),
            pattern: r"^OVERRIDDEN$".into(),
            kind: RuleKind::Chapter,
            enabled: true,
            priority: 1,
            source: RuleSource::User,
            description: "override".into(),
        });
        a.merge(b);
        let r = a.find("builtin-chapter-cn-zhang").unwrap();
        assert_eq!(r.pattern, "^OVERRIDDEN$");
        assert_eq!(r.source, RuleSource::User);
    }
}
