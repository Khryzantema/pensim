# Prompt para continuar no Claude Code CLI

Cole o bloco abaixo no Claude Code, rodando dentro de um clone do repositório
`Khryzantema/pensim` no ramo `claude/android-ballistic-simulation-game-kajd4w`.

---

```text
Você vai continuar o PenSim: um simulador 3D de impacto balístico em Rust (núcleo em
crates/core, app egui em crates/app, Android via cargo-apk e Linux). Responda e escreva
comentários/commits em português do Brasil. O dono quer qualidade acima de velocidade
(regra 70% qualidade / 30% velocidade, detalhada em docs/NOVO-NUCLEO.md, seção 1).

LEIA PRIMEIRO, inteiro: docs/NOVO-NUCLEO.md. É o projeto e o diário do núcleo novo
(seções 11 a 14 dizem o que já foi feito e medido).

ESTADO ATUAL
- Núcleo novo de elementos finitos explícitos com erosão em crates/core/src/fem/:
  mesh.rs (hexaedros; malha em O para esfera e sólidos de revolução), mod.rs (elemento de
  1 ponto + hourglass Flanagan-Belytschko, Jaumann, Johnson-Cook com retorno radial,
  Mie-Grüneisen, viscosidade artificial, erosão por dano JC/frágil/distorção), contact.rs
  (contato geral nó-contra-superfície com atrito e tratamento de penetração inicial),
  material.rs (parâmetros JC/EOS; from_library converte os materiais do app — valores
  típicos de literatura, NÃO calibrados), build.rs (Config do app → malha), run.rs (FemSim,
  mesma interface do simulador antigo).
- O app já usa o núcleo novo por padrão (Config::fem = true; lib.rs::Solver escolhe).
  A peridinâmica antiga (crates/core/src/sim.rs) continua como opção "Peridinâmica (antigo)".
- Visualização: faces direto dos hexaedros (crates/app/src/mesh.rs::from_hex), partículas =
  nós sem elemento vivo. Arquivo salvo .psim v5 guarda hexaedros e elementos vivos.
  Pré-visualização "Malha 3D" mostra a malha real. Pack de munições .ppack
  (crates/app/src/pack.rs) com exportação na aba Munição.
- Testes: cargo test --release (fem_verify.rs: onda, tração, energia, Taylor, malha;
  fem_impact.rs: esfera × chapa; save_fem.rs). Todos passam. Exemplos úteis:
  cargo run --release -p pensim-core --example fem_hg (diagnóstico de energia) e
  --example fem_app (munição da biblioteca no núcleo novo; AMMO, NODES, DUR, A).
- Capturas de tela do app: ganchos PENSIM_DEMO, PENSIM_FACES, PENSIM_SIDE, PENSIM_ISO,
  PENSIM_NODES, PENSIM_DUR, PENSIM_AT, PENSIM_WS, PENSIM_PEEK, PENSIM_NOPART (ver app.rs).
  O dono usa 16 nós no diâmetro e partículas ligadas: teste assim.

O QUE FALTA, NESTA ORDEM
1. Fase 3 — validação contra o OpenRadioss (docs/NOVO-NUCLEO.md, seções 6.2 e 14).
   a. Compilar o OpenRadioss nesta máquina: código em github.com/OpenRadioss/OpenRadioss
      (o dono tem um fork em Khryzantema/openradioss). A compilação precisa de extlib
      (bibliotecas prontas; ver EXTLIB_VERSION.json e Compiling_tools/script/load_extlib.py).
      O link oficial do extlib não abriu para o dono. Há uma cópia pública não oficial em
      github.com/xupeiwust/OpenRadioss_extlib (versão 59; o código pede 82) — é binário de
      terceiro não verificado: PERGUNTE ao dono antes de usar. Use a opção -open_reader
      (leitor aberto, dispensa o hm_reader da Altair):
        cd starter && ./build_script.sh -arch=linux64_gf -release -open_reader -no-python -nt=N
        cd engine  && ./build_script.sh -arch=linux64_gf -release -nt=N   (ver HOWTO.md)
      Se não compilar, alternativa: a versão já compilada para Linux nos releases oficiais.
   b. Escrever um gerador de decks .rad a partir do mesmo Config/malha do PenSim (mesmos nós
      e elementos, mesmos materiais JC + Mie-Grüneisen, contato TYPE7, erosão), para os 80
      casos: esfera de aço 4340 Ø10 mm × chapas RHA e Al 6061-T6, espessura 0,5D e 1D,
      chapa a 0°, 30°, 45°, 60°, 5 velocidades em torno do limite balístico.
   c. Rodar, extrair velocidade residual, diâmetro do furo, massa do tampão, instante de
      saída, perfurou/ricocheteou; salvar CSV em calib/ref/; comparar com o PenSim.
      Meta: velocidade residual dentro de 5% e mesmo desfecho. Se o hourglass dos elementos
      erodidos fizer diferença > 3%, considerar integração completa (seção 12).
   d. Estudo de convergência 8/12/16/24 elementos no diâmetro (seção 6.4).
2. Fase 5 — calibração contra ensaios (seção 6.3). Só três parâmetros podem ser ajustados:
   k da regularização pela malha, atrito, D1 do dano. Os números dos ensaios TÊM de vir dos
   artigos (Goldsmith & Finnegan 1971; Recht & Ipson 1963; Børvik et al.) — nunca de memória.
   Depois: ogivas e biblioteca de munição (M61 e .50 AP dentro de 10% do limite real).
3. Fase 6 — desempenho: estado em f32 (somas em f64), paralelismo melhor (o dono tem um
   Xeon e reclamou de uso baixo de CPU), depois GPU. Meta: 260 mil elementos, 1000 µs,
   < 2 min em 8 núcleos. Medir antes de otimizar.
4. Pendências menores: interfaces internas do projétil (camisa/núcleo) alinhadas na malha em
   O (hoje a peça vem pelo centróide do elemento); ponta aguda usa raio mínimo de ¼ do passo;
   AppImage (appimagetool pode precisar de permissão).

REGRAS
- Não invente números de ensaio nem de referência; se não tiver a fonte, diga.
- Rode os testes antes de cada commit; não desligue teste para passar.
- Commits pequenos e descritivos; push no ramo claude/android-ballistic-simulation-game-kajd4w.
- Ao entregar build: APK com `cargo apk build -p pensim-app --release` (LTO fat via
  CARGO_PROFILE_RELEASE_LTO=fat, CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1; precisa de
  ANDROID_HOME, NDK 27 e a keystore de release do dono) e o binário Linux
  `cargo build --release -p pensim-app --bin pensim`.
- Antes de tarefas longas, diga ao dono o que vai fazer; ao terminar, resuma o que mudou, o
  que foi medido e o que não foi testado.
```
