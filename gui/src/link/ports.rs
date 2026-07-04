//! Перечисление доступных последовательных портов — единственная точка, где gui
//! обращается к `ediabas::available_ports()`. UI получает список только как
//! `model::Update::Ports`, самого `ediabas` не видит.

/// Список доступных COM/tty-портов (кроссплатформенно).
pub fn available_ports() -> Vec<String> {
    ediabas::available_ports()
}
