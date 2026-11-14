use leptos::prelude::*;

/// Компактный чекбокс-переключатель, отрисованный знаком: выключен —
/// красный "−", включён — зелёный "+". Используется для режима хода
/// (уменьшить/увеличить) и режима взрывов (Lossy/Lossless).
#[component]
pub fn ModeCheckbox(
    /// Текущее состояние: `true` — включён (зелёный "+").
    checked: Signal<bool>,
    /// Записывает новое состояние по клику.
    set_checked: SignalSetter<bool>,
    /// Доступное имя (title/aria-label) текущего состояния.
    label: Signal<String>,
    /// Реактивный признак блокировки.
    #[prop(optional)]
    disabled: Option<Signal<bool>>,
) -> impl IntoView {
    let class = move || {
        if checked.get() {
            "mode-checkbox mode-checkbox--on"
        } else {
            "mode-checkbox mode-checkbox--off"
        }
    };
    let is_disabled = move || disabled.as_ref().is_some_and(|signal| signal.get());

    view! {
        <button
            type="button"
            class=class
            role="checkbox"
            aria-checked=move || checked.get().to_string()
            aria-label=move || label.get()
            title=move || label.get()
            disabled=is_disabled
            on:click=move |_| set_checked.set(!checked.get_untracked())
        >
            <span class="mode-checkbox__sign" aria-hidden="true">
                {move || if checked.get() { "+" } else { "−" }}
            </span>
        </button>
    }
}
