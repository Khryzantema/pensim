# Prompt para calibrar no PC (Claude Code CLI)

Cole o bloco abaixo no Claude Code, dentro de um clone do repositório `Khryzantema/pensim` no
ramo `claude/android-ballistic-simulation-game-kajd4w` (Linux; o PC do dono é um Xeon
E5-2696 v3, 18 núcleos / 36 threads, 64 GB).

---

```text
Você vai continuar a CALIBRAÇÃO do PenSim, um simulador 3D de impacto balístico em Rust.
Responda, comente e faça commits em português do Brasil. Regras do dono:
- 70% qualidade / 30% velocidade (docs/NOVO-NUCLEO.md, seção 1).
- UMA coisa de cada vez. Nas comparações, os dois solvers rodam o MESMO caso ao mesmo tempo
  (um PenSim + um OpenCourant), nunca vários casos em paralelo.
- Não invente números de ensaio. Não desligue teste para passar. Rode os testes antes de cada
  commit. Commits pequenos; push no ramo acima.

LEIA PRIMEIRO, inteiro: docs/NOVO-NUCLEO.md — principalmente as seções 11 a 16 (o que já foi
feito, medido e decidido). Resumo do estado:
- Núcleo novo de elementos finitos explícitos com erosão em crates/core/src/fem/ (malha em O,
  hexaedro de 1 ponto + hourglass, Johnson-Cook, Mie-Grüneisen, contato nó-superfície). O app usa
  ele por padrão. Testes: cargo test --release (fem_verify, fem_impact, save_fem: todos passam).
- Referência: OpenCourant (continuação comunitária do OpenRadioss, que foi descontinuado em
  01/10/2026). Pacote Linux pronto em https://opencourant.org/downloads/ (release
  latest-20261011, OpenCourant_linux64.zip). Descompactar e apontar OC=/caminho/OpenCourant.
- crates/core/src/fem/radioss.rs exporta a MESMA malha e materiais do PenSim para o formato do
  OpenCourant. Medida da velocidade: média por nó (massas em .mass), porque a massa por peça no
  histórico do Radioss perde os elementos apagados. Erosão por distorção igualada:
  /DT/BRICK/DEL e /DT/INTER/DEL a 20% do passo inicial (sem isso o OpenCourant aborta no contato).

FERRAMENTAS (calib/)
- calib/run_calib.py: esfera de aço 4340 Ø10 mm × chapas Al 6061-T6 e RHA, 5 e 10 mm, a 0°, 30°,
  45°, 60°, 5 velocidades por material (80 casos). Para cada caso: escreve o deck e roda PenSim e
  OpenCourant juntos; resultado em calib/ref/esfera_chapa.csv (pula casos já presentes).
  Filtros: MATS=al|rha, ANGS=0,60, VELS=700,1100, ND=12|16|24, CSV=outro.csv.
  Núcleos: PENSIM_THREADS e OC_THREADS (padrão: metade para cada).
- calib/one.sh: um caso só (MAT T A V ND FRIC DTFRAC ISOLID PENSIM_HG ...).
- calib/run_ammo.py: munições da biblioteca × RHA a 0°/30°/60° (SPIN=mil rpm para rotação).
  ATENÇÃO: hoje o OpenCourant aborta nesses casos por causa dos elementos minúsculos da ponta da
  ogiva (raio mínimo de ¼ do passo em mesh::revolution_x). Corrigir antes (item 3).
- Exemplos: examples/calib_case.rs, calib_ammo.rs, fem_app.rs (munição no núcleo novo),
  fem_hg.rs (energia por peça), meshq_ammo.rs (qualidade da malha).

ONDE ESTAMOS (seção 15 do documento): 80 casos, mesmo desfecho em 63/80, erro médio de 9,6% na
velocidade residual quando há perfuração clara (meta: 5%). Energia do PenSim < 1,1%.
Desvios que sobram:
 A. Perto do limite balístico a chapa do PenSim cede antes (ex.: Al 10 mm a 700 m/s: PenSim 404,
    OpenCourant −60 m/s; RHA 5 mm a 1200 m/s; RHA 10 mm a 1800 m/s).
 B. RHA a 60°: PenSim 500–600 m/s, OpenCourant 640–780 m/s.

TAREFAS, NESTA ORDEM, UMA DE CADA VEZ
1. Baseline no PC: rodar calib/run_calib.py com ND=12 e conferir que bate com
   calib/ref/esfera_chapa.csv (mesmos números dentro de ~1%). Medir o tempo.
2. Desvio A, hipóteses UMA por vez, sempre no mesmo caso curto (Al 10 mm, 0°, 700 m/s via
   calib/one.sh), e só depois confirmar nos 80:
   a. Elemento/hourglass: OpenCourant com ISOLID=1 (1 ponto, hourglass viscoso) em vez de 24; e
      PenSim com PENSIM_HG=0.05/0.2. Se for isso, o PenSim precisa de hourglass melhor
      (ex.: estabilização física tipo Belytschko-Bindeman) ou integração completa na chapa.
   b. Erosão: no PenSim, EROSION_DT_FRAC (fem/mod.rs) 0,1 vs 0,2; no OpenCourant DTFRAC.
   c. Dano: conferir triaxialidade/taxa/temperatura do dano Johnson-Cook do PenSim contra o
      manual do Radioss (/FAIL/JOHNSON) — mesmas definições?
   d. Contato: penalidade/offset de penetração inicial (fem/contact.rs).
   Registrar cada resultado no documento (seção nova) antes de mudar a próxima coisa.
3. Ponta da ogiva: engrossar o raio mínimo da ponta em mesh::revolution_x (ou cortar a ponta
   num meplat de ~½ elemento) para o OpenCourant não abortar; rodar calib/run_ammo.py.
4. Rotação (seção 16): repetir .50 BMG × 20 mm RHA a 0/30/45/60°, com e sem SPIN, ND 16, e
   comparar com o OpenCourant (o giro já entra no deck pela velocidade inicial por nó).
5. Núcleo AP frágil: hoje o aço "projétil AP" rompe só com ~30% de deformação (parâmetros
   genéricos em fem/material.rs::from_library). NÃO chutar: pedir ao dono os dados ou artigos.
6. Convergência: repetir os 80 casos com ND=16 e ND=24 (o dono usa 16 no app).

Ao terminar cada tarefa: diga ao dono, em poucas linhas, o que mudou, o que foi medido e o que
não foi testado; atualize docs/NOVO-NUCLEO.md; commit e push.
```
