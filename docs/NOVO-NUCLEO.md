# PenSim — projeto do novo núcleo de simulação

Documento de passagem. Quem continuar deve conseguir trabalhar só com este arquivo e o código.
Estado: **só projeto, nada disto está implementado.**

## 1. Decisão

Jogar fora o núcleo de peridinâmica (`crates/core/src/sim.rs`) e escrever um núcleo novo de
**elementos finitos explícitos Lagrangianos com erosão**, o mesmo tipo de método do
Ansys Autodyn/LS-DYNA. Fica: a interface, o designer de desenho, a biblioteca de munição e
materiais, o formato de projeto.

Código de referência: **OpenRadioss** (Altair, AGPL-3.0, github.com/OpenRadioss/OpenRadioss).

Por que ele, e não os outros:

| Candidato | Por que não |
|---|---|
| Peridigm, LAMMPS-PERI | É peridinâmica de novo: mesmos problemas (nuvem de fragmentos, malha de faces reconstruída) |
| Uintah (MPM), PySPH, Taichi-MPM | Sem malha: a superfície tem de ser reconstruída, que é o que deu errado aqui |
| Yade (DEM) | Não modela metal dúctil |
| **OpenRadioss** | Solver industrial de impacto, manual de teoria público, tem Johnson-Cook, erosão, contato, SPH |

Os três defeitos que motivaram a troca somem pelo método, não por remendo de desenho:
- **Malha com buracos / pixelada**: a superfície é a face externa dos elementos. Não há
  reconstrução, logo não há buraco nem triângulo solto.
- **Camada de fragmentos entre munição e alvo**: elemento que falha é apagado (erosão).
- **Munição não arredondada**: os nós da borda são encaixados no perfil real (seção 4).

### O que "usar de base" quer dizer aqui

OpenRadioss é Fortran, ~1,5 milhão de linhas, e não roda em Android. Portanto:
1. **Reimplementar em Rust** os algoritmos, a partir do manual de teoria e dos artigos originais.
2. **Usar o OpenRadioss como gabarito**: rodar os mesmos casos nos dois e comparar.

Não copiar código dele. Copiar obriga o PenSim inteiro a ser AGPL; o repositório hoje não tem
arquivo de licença, e essa escolha é do dono do projeto.

## 2. O que fica e o que sai

| Arquivo | Destino |
|---|---|
| `crates/app/src/{app,draw,sketch,view3d,plot,worker}.rs` | Fica. `app.rs` muda só onde lê quadros |
| `crates/app/src/mesh.rs` | **Sai.** Vira ~80 linhas: faces externas dos elementos vivos |
| `crates/core/src/{config,ammo,save,store,sweep}.rs` | Fica. `save.rs` sobe para VERSION 5 |
| `crates/core/src/material.rs` | Fica e ganha parâmetros (seção 5) |
| `crates/core/src/geometry.rs` | Reescrito: gera hexaedros em vez de nós |
| `crates/core/src/sim.rs` | **Sai.** Substituído por `crates/core/src/fem/` |
| `crates/core/tests/physics.rs`, `examples/*` | Reescritos para o núcleo novo |

Manter a peridinâmica atrás de uma opção não vale a pena: duplica o trabalho de visualização.
Apagar depois que a fase 3 passar.

## 3. Interface entre núcleo e app

O app hoje usa de `Sim`: `new`, `advance_for`, `done`, `finish_now`, `progress`, `outcome`,
`frames`, `part_ids`, `t_end`, `time`, `dt`. Manter esses nomes para o app quase não mudar.

Muda o conteúdo do quadro:

```rust
pub struct Topology {            // fixo por simulação
    pub hex: Vec<[u32; 8]>,      // conectividade
    pub part: Vec<u8>,           // peça de cada elemento
    pub node0: Vec<[f32; 3]>,    // posição inicial
}
pub struct Frame {               // por instante, comprimido como hoje (store.rs)
    pub time: f64,
    pub pos: Vec<[f32; 3]>,      // por nó
    pub alive: BitVec,           // por elemento
    pub fields: [Vec<f32>; 5],   // por elemento: von Mises, pressão, def. plástica, dano, temperatura
    pub debris: Vec<u32>,        // nós sem elemento vivo (as "partículas")
}
```

Visualização: face é externa se pertence a exatamente um elemento vivo. Normal por vértice =
média das faces. Cor por vértice = média dos elementos vizinhos. Corte de seção = descartar
elementos de um lado do plano. O chip "Partículas" passa a ligar/desligar `debris`.

## 4. Malha

Hexaedros de 8 nós a partir das formas que `geometry.rs` já descreve.

1. **Grade estruturada** com passo `dx` (o "nós no diâmetro" da interface continua valendo).
   Um hexaedro entra se o centro está dentro da peça.
2. **Encaixe da borda**: cada nó de superfície é projetado na superfície real (perfil de
   revolução do projétil, plano da chapa, polígono extrudado dos objetos). É isso que deixa a
   esfera e a ogiva redondas em vez de em degraus.
3. **Qualidade**: depois do encaixe, 2–3 passadas de suavização só nos nós internos vizinhos
   da borda; rejeitar a malha se algum jacobiano ≤ 0,2 do nominal.
4. **Interfaces entre peças do projétil** (camisa/núcleo/capuz): nós compartilhados. Cada
   elemento leva o material da peça; a separação acontece por erosão.
5. **Simetria** 1/2 e 1/4: condição de contorno nos nós do plano (velocidade normal = 0).
   Os nós-fantasma atuais somem.

Risco conhecido: peças mais finas que `dx` (camisa de bala). Regra: espessura mínima de 1
elemento, com aviso na interface quando a peça foi engrossada.

## 5. Física

Tudo abaixo é padrão em códigos de impacto. Referência de fórmulas: manual de teoria do
Radioss e os artigos citados.

**Elemento.** Hexaedro de 1 ponto de integração com controle de hourglass
Flanagan-Belytschko (viscoso + rigidez). Energia de hourglass tem de ficar < 10% da interna;
é saída de diagnóstico obrigatória.

**Integração.** Diferenças centrais, `dt = 0,9 · min(L_e / c_e)` recalculado a cada passo.
Massa concentrada nos nós. Viscosidade artificial linear + quadrática (von Neumann-Richtmyer).

**Tensão.** Taxa de Jaumann. Parte desviadora: Johnson-Cook com retorno radial
`σy = (A + B εp^n)(1 + C ln ε̇*)(1 − T*^m)`. Parte volumétrica: Mie-Grüneisen
(`c0`, `s`, `Γ0`) para metais; linear para o resto.

**Falha.** Dano de Johnson-Cook `D = Σ Δεp / εf`, `εf = (D1 + D2 e^{D3 σ*})(1 + D4 ln ε̇*)(1 + D5 T*)`.
`D ≥ 1` apaga o elemento. Segundo critério, só para frágeis (cerâmica, WC, vidro-textolite):
tensão principal máxima. Terceiro, de segurança: deformação geométrica extrema ou `dt` do
elemento abaixo de 1% do inicial.

**Regularização pela malha — obrigatório.** A deformação de falha depende do tamanho do
elemento. Sem corrigir, o resultado muda quando o usuário muda "nós no diâmetro". Escalar
`εf` por `(L_e / L_ref)^(-k)` e calibrar `k` no estudo de convergência da fase 3.

**Contato.** Algoritmo *pinball* (Belytschko & Neal, 1991): uma esfera inscrita em cada
elemento de superfície, penalidade quando esferas de corpos diferentes se sobrepõem, atrito
de Coulomb. Motivos: é simples, paraleliza bem, e a superfície recém-exposta pela erosão
entra no contato sem refazer nada. Busca por grade uniforme (reaproveitar a ideia do `HGrid`).
Inclui autocontato do alvo (o tampão contra a parede do furo).

**Elementos apagados.** A massa fica nos nós. Nó sem nenhum elemento vivo vira partícula
livre: guarda massa e quantidade de movimento, participa do contato como esfera. Opção
"apagar partículas" para ficar igual ao Autodyn.

**Materiais.** `Material` ganha: `jc: {a, b, n, c, m}`, `jc_dmg: [f64; 5]`, `eos: {c0, s, gamma}`,
`poisson`, `brittle_stress`. Hoje `jc_damage()` e `poisson()` em `sim.rs` escolhem valores pelo
*nome* do material; isso acaba, os números vão para a tabela. `rha_bhn`/`adjust_rha` continuam.

## 6. Calibração com esferas

Esfera primeiro porque não tem orientação, a geometria é exata e há décadas de ensaios.
Só depois de fechar esfera é que se mexe em ogiva e munição real.

### 6.1 Verificação (o código está certo?) — sem ajuste de parâmetro

| Teste | Critério |
|---|---|
| Onda elástica em barra | Velocidade da frente dentro de 2% de `√(E/ρ)` |
| Cubo em tração/compressão uniforme | Tensão dentro de 1% da analítica |
| Barra de Taylor (cilindro de cobre contra parede rígida) | Comprimento final e diâmetro do cogumelo dentro de 5% do OpenRadioss |
| Conservação | Erro de energia total < 2%; hourglass < 10% da interna |
| Simetria | Impacto normal em 1/4, 1/2 e modelo inteiro dão a mesma velocidade residual (±1%) |

### 6.2 Código contra código (PenSim × OpenRadioss) — mesma malha, mesmos materiais

Esfera de aço Ø 10 mm. Chapas: RHA e alumínio 6061-T6, espessura 0,5 D e 1 D.
Ângulos da chapa: **0°, 30°, 45°, 60°**. Velocidades: 5 pontos em torno do limite balístico.
Total: 2 × 2 × 4 × 5 = 80 casos.

Comparar: velocidade residual, diâmetro do furo, massa do tampão, instante de saída, e se
perfurou/ricocheteou. Meta: velocidade residual dentro de 5% e mesmo desfecho em todos.

Um script gera o deck do OpenRadioss (`.rad`) a partir do mesmo `Config`, para as duas malhas
serem idênticas. Resultados entram no repositório como CSV em `calib/ref/`.

### 6.3 Código contra ensaio — aqui se ajustam parâmetros

Séries candidatas. **Os números têm de ser tirados dos artigos; eu não os conferi e não
escrevi nenhum de memória aqui.**
- Goldsmith & Finnegan (1971), esferas de aço duro contra chapas de aço e alumínio, impacto normal.
- Backman & Finnegan / Finnegan et al., mesma linha de ensaios com chapa inclinada e ricochete.
- Recht & Ipson (1963), velocidade residual, normal e oblíquo; dá também a fórmula de checagem
  `v_r = a (v_i^p − v_bl^p)^(1/p)`.
- Børvik et al., Weldox 460 E 12 mm (cilindros, não esferas): serve para a fase de ogivas.

Parâmetros que podem ser mexidos, e só estes: `k` da regularização, atrito, `D1` do dano por
material. Tudo o mais vem de literatura. Calibrar em metade dos casos, validar na outra metade.
Meta: limite balístico dentro de 10%, ângulo crítico de ricochete dentro de 5°.

### 6.4 Convergência

Repetir 6.2 com 8, 12, 16 e 24 elementos no diâmetro. O limite balístico não pode variar mais
de 5% entre 12 e 24. O usuário trabalha com 16: esse é o padrão dos testes.

## 7. Fases e critério de pronto

| Fase | Entrega | Pronto quando |
|---|---|---|
| 1 | `fem/`: malha, elemento, JC, EOS, integração. Sem contato, sem falha | Passa 6.1 exceto Taylor |
| 2 | Contato pinball, erosão, partículas | Passa Taylor; esfera perfura chapa sem explodir energia |
| 3 | OpenRadioss rodando os 80 casos; comparação automática | Passa 6.2 e 6.4 |
| 4 | `Frame` novo, `mesh.rs` novo, `save.rs` v5, app ligado no núcleo novo | App roda uma simulação de ponta a ponta no Linux |
| 5 | Calibração 6.3; depois ogivas e a biblioteca de munição | Metas de 6.3; M61 e .50 AP dentro de 10% do limite real |
| 6 | Desempenho: SoA em `f32`, paralelismo por cores de elemento, depois GPU (wgpu) | 260 mil elementos, 1000 µs, em menos de 2 min num desktop de 8 núcleos |

Ordem importa: a fase 4 só começa depois da 3. Ligar a interface num núcleo não validado foi
o que deixou a versão atual difícil de corrigir.

## 8. Riscos

- **Rodar o OpenRadioss neste ambiente.** O classificador de permissões já bloqueou a execução
  de um binário baixado (`appimagetool`). Saídas: compilar do código-fonte com gfortran, ou o
  dono do projeto rodar os decks na máquina dele e commitar os CSV.
- **Contato é a parte que mais dá errado.** Se o pinball deixar penetração visível, trocar por
  nó-contra-superfície (interface TYPE7 do Radioss) só no par projétil-alvo.
- **Erosão subestima penetração em chapa grossa** (massa some da frente do projétil). É por isso
  que a massa fica nos nós; conferir no caso de 2 D de espessura.
- **Hexaedro de 1 ponto em casca fina** (camisa com 1 elemento) trava ou faz hourglass. Se
  aparecer, usar integração completa só nessas peças.
- **Custo.** Estimativa não medida: um hexaedro custa menos que os ~120 vínculos por nó da
  peridinâmica, então espero ficar mais rápido que hoje. Medir na fase 2.

## 9. Estado do código na hora da passagem

- Ramo `claude/android-ballistic-simulation-game-kajd4w`, tudo commitado.
- A interface já tem: duração em µs (padrão 1000), chip "Partículas", botões de vista no menu
  Visualização, salvar/abrir projéteis desenhados (`.pdes`), material Glass Textolite.
- Ganchos de captura de tela: `PENSIM_DEMO`, `PENSIM_FACES`, `PENSIM_SIDE`, `PENSIM_NODES`,
  `PENSIM_DUR`, `PENSIM_AT`, com `xvfb-run` e `import`. Testar sempre com `PENSIM_NODES=16`
  e partículas ligadas, que é como o usuário usa.
- Último ajuste em `mesh.rs` (esconder fragmentos do projétil colados nele) compila e passa
  nos testes, mas **não foi conferido em captura de tela**. Vai ser descartado com o `mesh.rs`.
- AppImage: não gerado, bloqueado pelo classificador. Entrega atual é `.tar.gz`.
