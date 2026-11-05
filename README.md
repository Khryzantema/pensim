# PenSim — simulação de impacto balístico 3D (Rust · Android)

Simulador 3D de impacto balístico com interface estilo CAD/CAE (SolidWorks/Fusion + contornos tipo Ansys).

* **`crates/core`** – solver: peridinâmica *bond-based* 3D (ν = 1/4) com plasticidade de ligação,
  encruamento, amolecimento térmico (Johnson-Cook) com aquecimento adiabático, fratura por estiramento
  crítico e contato por penalidade. Passo explícito **multithread (rayon)**, sem condições de corrida
  (formulação *gather* por nó). Balanço de energia reportado (erro < ~1% nos testes).
* **`crates/app`** – UI egui/eframe: ribbon, navegador de recursos, propriedades, **CAD simples**
  (esboço de perfil → revolução para o projétil; camadas extrudadas para o alvo), viewport 3D com
  órbita/pinça/corte de seção, contornos (von Mises, pressão, deformação plástica, temperatura, dano,
  velocidade), gráficos de energia e **varredura paralela do limite balístico (V50)**.

> Modelo didático/qualitativo. **Não é um código validado** — não use para projeto de blindagem real.

## Rodar no desktop
```
cargo run --release -p pensim-app
cargo test --release -p pensim-core
```

## APK Android
O workflow `.github/workflows/android.yml` compila o APK (arm64) com `cargo-apk` e publica em **Releases**.
Localmente (precisa de Android SDK + NDK):
```
rustup target add aarch64-linux-android
cargo install cargo-apk
export ANDROID_HOME=... ANDROID_NDK_ROOT=...
cargo apk build --release -p pensim-app
```
Em celulares, prefira malha "média/rápida" (≈20 mil nós); malhas finas custam muito mais.
