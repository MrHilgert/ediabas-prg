//! Слой РАБОТЫ С ЭБУ (ediabas + inpa, НЕ egui).
//!
//! Владеет блокирующей `ediabas::Session` на отдельном потоке и общается с UI только
//! через `crate::model::{Intent, Update}` по двум mpsc-каналам. Про отрисовку ничего
//! не знает: ни `egui`, ни цветов, ни разметки. Единственная связь с UI-циклом —
//! wake-замыкание (`Box<dyn Fn()`), которым поток будит перерисовку, не называя egui.
//!
//! - `Worker` — хэндл, за который держится UI: шлёт `Intent`, дренирует `Update`.
//! - `decode` — превращение `JobResult` в view-модели.
//! - `ports` — перечисление последовательных портов.

mod decode;
mod ports;
mod worker;

pub use worker::Worker;
