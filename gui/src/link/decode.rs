//! Декодирование сырого `ediabas::JobResult` в plain view-модели (`crate::model`).
//!
//! Это единственное место, где данные ЭБУ превращаются в то, что рисует `ui/`.
//! Импортирует `ediabas` + `inpa` (домен экранов), но НЕ `egui`: здесь нет ни цветов,
//! ни разметки — только «какое значение у какой строки экрана».

use ediabas::JobResult;
use inpa::model::{Row, Screen};

use crate::model::{DtcView, FaultView, MeasCell, MeasFrame, UwView};

/// Декодировать `FS_LESEN` в список DTC (одна запись на набор с `F_ORT_NR`).
/// Перенос `session::faults::parse_dtcs` без изменения логики.
pub fn decode_faults(fr: &JobResult) -> FaultView {
    let mut dtcs = Vec::new();
    for set in fr.sets() {
        if !set.contains("F_ORT_NR") {
            continue; // напр. хвостовой набор JOB_STATUS
        }
        let code_num = set.get_i64("F_ORT_NR").unwrap_or(0);
        let text = set.get_str("F_ORT_TEXT").unwrap_or_default().trim().to_string();
        if code_num == 0 && text.is_empty() {
            continue;
        }
        // Стоп-кадр: F_UW{i}_TEXT/_WERT/_EINH, подряд с 1.
        let mut uw = Vec::new();
        for i in 1..=64 {
            let tkey = format!("F_UW{i}_TEXT");
            if !set.contains(&tkey) {
                break;
            }
            let val = set
                .get_f64(&format!("F_UW{i}_WERT"))
                .map(fmt_uw)
                .or_else(|| set.get_str(&format!("F_UW{i}_WERT")))
                .unwrap_or_default();
            uw.push(UwView {
                text: set.get_str(&tkey).unwrap_or_default().trim().to_string(),
                val,
                unit: set.get_str(&format!("F_UW{i}_EINH")).unwrap_or_default().trim().to_string(),
            });
        }
        // Причины/типы: F_ART{i}_TEXT. Пропускаем "--" и старшие статус-флаги (NR >= 0xEB).
        let mut causes = Vec::new();
        let mut sporadic = false;
        for i in 1..=64 {
            let key = format!("F_ART{i}_TEXT");
            if !set.contains(&key) {
                break;
            }
            let nr = set.get_i64(&format!("F_ART{i}_NR")).unwrap_or(0);
            let txt = set.get_str(&key).unwrap_or_default().trim().to_string();
            if txt.to_lowercase().contains("sporadisch") {
                sporadic = true;
            }
            if nr > 0 && nr < 0xEB && !txt.is_empty() && txt != "--" {
                causes.push(txt);
            }
        }
        dtcs.push(DtcView {
            code: format!("{:04X}", (code_num as u32) & 0xffff),
            text,
            present: set.get_i64("F_VORHANDEN").unwrap_or(0) != 0,
            sporadic,
            raw: set.get_str("F_HEX_CODE").unwrap_or_default().trim().to_string(),
            hfk: set.get_i64("F_HFK").unwrap_or(0),
            lz: set.get_i64("F_LZ").unwrap_or(0),
            uw_satz: set.get_i64("F_UW_SATZ").unwrap_or(0),
            uw,
            causes,
        });
    }
    FaultView { dtcs }
}

/// Декодировать опрошенный `JobResult` в позиционный кадр значений для `screen`.
/// Одна ячейка на строку экрана, у которой есть result-имя; ключ — индекс строки в
/// `screen.rows` (тот же порядок, в котором `ui/` их рисует). Перенос `session::row_value`.
pub fn decode_live(fr: &JobResult, screen: &Screen) -> MeasFrame {
    let mut cells = Vec::new();
    for (i, row) in screen.rows.iter().enumerate() {
        let Some(res) = row.result() else { continue };
        let cell = match row {
            Row::Analog { unit, .. } => {
                // Динамическая единица (result `_EINH`) с приоритетом над статической.
                let u = res
                    .strip_suffix("_WERT")
                    .map(|b| format!("{b}_EINH"))
                    .and_then(|k| fr.get_str(&k))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| unit.clone());
                MeasCell { row: i, num: fr.get_f64(res), text: None, unit: u }
            }
            Row::Logical { .. } => {
                MeasCell { row: i, num: fr.get_f64(res), text: None, unit: String::new() }
            }
            Row::Text { .. } => {
                MeasCell { row: i, num: None, text: fr.get_str(res), unit: String::new() }
            }
            _ => continue,
        };
        cells.push(cell);
    }
    let raw_names = fr.sets().iter().flat_map(|s| s.names()).map(|n| n.to_string()).collect();
    MeasFrame { cells, raw_names }
}

fn fmt_uw(v: f64) -> String {
    if v.fract().abs() < 0.05 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}
