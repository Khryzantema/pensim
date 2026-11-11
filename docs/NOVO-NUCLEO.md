# PenSim — projeto do novo núcleo de simulação

Documento de passagem. Quem continuar deve conseguir trabalhar só com este arquivo e o código.
Estado: **fases 1 e 2 concluídas; fase 4 adiantada** (app já usa o núcleo novo) a pedido do dono, para poder testar. Fase 3 (OpenRadioss) pendente: precisa liberar github.com na rede do ambiente.

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

**Licença: o PenSim é AGPL-3.0-or-later** (arquivo `LICENSE`), decidido pelo dono do projeto
("faz do jeito que quiser"). É a mesma do OpenRadioss, então é permitido ler e portar trechos
dele quando o manual não bastar. Portar para Rust continua sendo o caminho normal; ao portar um
trecho, citar o arquivo de origem no comentário.

### Critério de escolha entre alternativas

Pedido do dono: **70% qualidade, 30% velocidade.** Na dúvida entre duas opções, vale a mais
fiel, desde que não custe mais que ~3× o tempo da alternativa barata. Acima disso, fica a
barata e a cara vira opção "Preciso" na interface. Ganho de velocidade que piore um resultado
da seção 6 em mais de 2–3% não entra.

Escolhas que isso fixa (valem sobre o resto do documento onde houver conflito):
- **Elemento**: hexaedro de 1 ponto com hourglass por rigidez (é o padrão da indústria para
  impacto; integração completa trava em plasticidade e não é "mais qualidade"). Integração
  completa só em peças com 1 elemento de espessura.
- **Contato projétil-alvo**: nó-contra-superfície (tipo TYPE7 do Radioss), que não deixa folga
  nem penetração visível. Pinball fica para partículas e autocontato do alvo.
- **Precisão**: estado em `f32`, somas de força e energia em `f64`.
- **Malha**: encaixe da borda no perfil real sempre ligado.
- **Falha**: Johnson-Cook completo (5 parâmetros) com regularização pela malha; nada de
  critério simplificado de deformação máxima.

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
- **Contato é a parte que mais dá errado.** Nó-contra-superfície com erosão exige refazer a
  lista de faces expostas a cada elemento apagado; se isso atrasar a fase 2, começar com pinball
  em tudo e trocar o par projétil-alvo antes da fase 3.
- **Erosão subestima penetração em chapa grossa** (massa some da frente do projétil). É por isso
  que a massa fica nos nós; conferir no caso de 2 D de espessura.
- **Hexaedro de 1 ponto em casca fina** (camisa com 1 elemento) trava ou faz hourglass. Se
  aparecer, usar integração completa só nessas peças.
- **Custo.** Estimativa não medida: um hexaedro custa menos que os ~120 vínculos por nó da
  peridinâmica, então espero ficar mais rápido que hoje. Medir na fase 2.

## 9. Pack de munições pago

Único requisito comercial do dono: vender um pack de munições desenhadas por ele. O programa é
aberto; o pack é conteúdo dele, fora do repositório, com licença própria.

Regras de projeto para manter essa separação limpa:
- **O pack é só dados**, carregado em tempo de execução. Nunca compilado no binário, nunca com código.
- **Formato `.ppack`**: um arquivo zstd contendo um índice e N munições. Cada munição = o texto
  `.pdes` que o designer já grava (`Drawing::to_text`) + metadados: nome, grupo, calibre,
  velocidade de boca, massa real, nota. Primeira linha: `PENSIM-PACK 1`, nome do pack, autor,
  texto de licença do pack.
- **Onde fica**: pasta `packs/` ao lado de `projeteis/` (`saved_dir()` em `app.rs`). O app lista
  cada pack como um grupo no seletor de munição, abaixo da biblioteca gratuita.
- **Ferramenta de empacotar** no próprio app: "Exportar meus projéteis salvos como pack".
- **Sem trava no programa.** A AGPL não deixa restringir o código, e trava em arquivo de dados
  é fácil de contornar. A proteção do pack são os termos de venda.
- **Biblioteca gratuita** (`ammo.rs`) continua no repositório sob AGPL. O que o dono quiser
  vender não pode ser commitado aqui.
- No Android não há seletor de arquivos: o pack é copiado para a pasta de dados externa do app,
  como já acontece com as imagens de fundo.

Entra na fase 4, junto com a ligação do app ao núcleo novo.

## 10. Estado do código na hora da passagem

- Ramo `claude/android-ballistic-simulation-game-kajd4w`, tudo commitado.
- A interface já tem: duração em µs (padrão 1000), chip "Partículas", botões de vista no menu
  Visualização, salvar/abrir projéteis desenhados (`.pdes`), material Glass Textolite.
- Ganchos de captura de tela: `PENSIM_DEMO`, `PENSIM_FACES`, `PENSIM_SIDE`, `PENSIM_NODES`,
  `PENSIM_DUR`, `PENSIM_AT`, com `xvfb-run` e `import`. Testar sempre com `PENSIM_NODES=16`
  e partículas ligadas, que é como o usuário usa.
- Último ajuste em `mesh.rs` (esconder fragmentos do projétil colados nele) compila e passa
  nos testes, mas **não foi conferido em captura de tela**. Vai ser descartado com o `mesh.rs`.
- AppImage: não gerado, bloqueado pelo classificador. Entrega atual é `.tar.gz`.

## 11. Progresso da fase 1 — concluída

Todos os testes de `crates/core/tests/fem_verify.rs` passam:

| Teste | Resultado |
|---|---|
| Onda confinada (√((K+4G/3)/ρ)) | erro 0,42% |
| Onda em barra livre (√(E/ρ)) | erro 0,25% |
| Tração elástica (σ = Eε) | erro 0,05%, σyy/σxx 7e-7 |
| Tração plástica perfeita (σ = σy) | erro 0,00% |
| Corpo livre, Johnson-Cook + Mie-Grüneisen, εp até 0,69 | energia 1,6%, hourglass 2,1% da interna |
| Cilindro de cobre a 190 m/s contra parede (geometria de Taylor), inteiro / ½ / ¼ | idênticos; energia 0,04%; hourglass 1,7% |
| Malha da esfera, 8 a 24 no diâmetro | jacobiano escalonado mín. 0,67–0,68; volume −0,75% com 24 |

Malha: **em O** (núcleo cúbico arredondado + blocos de casca costurados nas diagonais), em
`mesh::sphere` e `mesh::revolution_x` (seção em O extrudada, raio por camada). A primeira
tentativa (grade em degraus com encaixe, depois um cubo único mapeado na bola) dava elementos
achatados ou invertidos: um cubo único deixa nós da superfície com ângulo de 180°.

Taylor deu L = 17,11 mm e raio máximo 6,55 mm (10 elementos no diâmetro). A comparação com o
OpenRadioss, que é o critério da tabela 6.1, fica para a fase 3; ainda não foi feita.

Pendências conhecidas para a fase 2:
- `revolution_x` usa raio mínimo de ¼ do passo na ponta; conferir com ogivas agudas.
- Interfaces internas (camisa/núcleo) ainda não: a casca do O tem de ter uma camada exatamente
  no raio da interface.
- Estado em `f64`; `f32` só na fase 6.

## 12. Progresso da fase 2 — concluída

Implementado em `fem/contact.rs` e `fem/mod.rs`:
- **Erosão**: dano de Johnson-Cook (com triaxialidade, taxa e temperatura), ruptura frágil por
  tensão principal máxima, distorção (passo do elemento < 10% do inicial; o documento pedia 1%,
  mas 1% deixaria o passo global cair 100×) e volume invertido. Contadores por motivo em
  `Fem::erosion_reasons()`.
- **Partículas**: nó sem elemento vivo mantém massa e velocidade e entra no contato.
- **Contato geral nó-contra-superfície** (inclui autocontato), penalidade
  k = max(0,1·K·A²/V, 0,1·m/dt²), atrito de Coulomb limitado. Penetração que não pode ter surgido
  no passo (face recém-exposta por erosão) vira referência e não gera força — sem isso o contato
  injetava ~8% de energia. Trabalho do contato contabilizado com a força média (n, n+1), como
  nos elementos.

Esfera de aço 4340 Ø10 mm × chapa 6061-T6 de 5 mm, 1000 m/s, borda presa (`fem_impact.rs`):

| Caso | v residual | Energia | HG/IE vivos |
|---|---|---|---|
| ¼ de modelo, 12 no diâmetro | 842 m/s | 0,25% | 3,1% |
| ½ de modelo, 12 no diâmetro | 838 m/s | 0,30% | 2,5% |
| ¼, 8 / 16 / 24 no diâmetro | 833 / 843 / 819 m/s | 0,39 / 0,21 / 0,12% | 3,8 / 2,3 / 1,6% |
| ½, chapa a 45° / 60° / 70° | 712 / 560 / 426 m/s (componente x) | < 0,5% | < 4,5% |

**Hourglass dos elementos erodidos**: 22–33% da energia interna deles. Medido: dobrar ou
quadruplicar a rigidez de hourglass não reduz isso e muda a velocidade residual em ±1%. É a
deformação real dos elementos que estão rompendo sob carga pontual do contato. Decisão (regra
70/30): manter o elemento de 1 ponto; o critério de 10% vale para os elementos vivos, e a energia
dos erodidos é reportada à parte (como o LS-DYNA faz). Integração completa só se a fase 3 mostrar
diferença > 3% contra o OpenRadioss. `PENSIM_HG` (variável de ambiente) muda a rigidez para
experimentos.

Desempenho medido (4 núcleos, `f64`): 64 mil elementos, 1471 passos em 27 s.

Os números de velocidade residual ainda **não foram comparados com nada externo**; isso é a fase 3.

## 13. Fase 4 adiantada — app ligado ao núcleo novo

O dono pediu para testar antes da validação; a fase 3 está parada porque o proxy de rede bloqueia
o download do OpenRadioss (github.com/codeload.github.com). Feito:
- `fem/build.rs`: `Config` → malha (projétil por revolução em O no eixo de voo, peças pelo
  `part_at` do projeto, vazio removido; camadas em blocos, discos em degraus; objetos desenhados em
  degraus; borda presa; ângulo e simetria com as mesmas convenções do simulador antigo).
- `fem/material.rs::from_library`: material da biblioteca → Johnson-Cook + EOS (valores típicos
  de literatura por tipo; **não calibrados**).
- `fem/run.rs::FemSim`: mesma interface do `Sim` antigo; `lib.rs::Solver` escolhe pelo
  `Config::fem` (padrão: novo). Varredura de V50 também usa o `Solver`.
- Quadro ganhou `alive` (bits dos elementos vivos). App: no modo faces, com hexaedros, a superfície
  sai de `mesh::from_hex` (faces externas dos elementos vivos visíveis, cópias espelhadas, corte de
  seção), sem a reconstrução antiga. Faces ligadas por padrão. Partículas = nós sem elemento vivo.
- Aba Simular: "Elementos finitos (novo)" / "Peridinâmica (antigo)".

Medido: 75 mm PzGr 39/42, 16 no diâmetro, 1000 µs: 36 mil elementos (¼), 50 s em 4 núcleos,
energia 0,16%.

Feito depois:
- Arquivo salvo v5: guarda hexaedros e elementos vivos por quadro; simulação aberta de arquivo
  aparece com faces (teste `crates/core/tests/save_fem.rs`).
- Pré-visualização ("Malha 3D") mostra a malha de hexaedros real com faces.
- Pack de munições `.ppack` (`crates/app/src/pack.rs`, formato no cabeçalho do arquivo): pasta
  `packs/` ao lado de `projeteis/`; cada pack vira um cartão na aba Munição; "Exportar como pack"
  junta os projéteis salvos com nome, autor e licença. O `.pdes` passou a guardar a velocidade.

## 14. Fase 3 — estado do OpenRadioss

- Código: fork do dono em `khryzantema/openradioss`, clonado em `/home/user/khryzantema/openradioss`
  (o GitHub da sessão só serve repositórios do dono; o original de terceiros é recusado).
- **Bloqueio**: a compilação baixa `extlib.zip` (bibliotecas prontas, incluindo o leitor de
  decks `hm_reader`, binário da Altair) de um release de `OpenRadioss/OpenRadioss_extlib`
  (`EXTLIB_VERSION.json`). O download é recusado pelo mesmo motivo, e fork não leva releases.
  Saídas: o dono baixar `extlib.zip` (ou o OpenRadioss já compilado para Linux) e colocar no fork
  dele num commit; ou rodar os decks na máquina dele e commitar os CSV em `calib/ref/`.

**Atualização (10/11/2026):** o OpenRadioss foi descontinuado pela Siemens em 01/10/2026 e o
repositório oficial saiu do GitHub (por isso o extlib dava 404). Continuação comunitária:
**OpenCourant** (https://opencourant.org, github.com/OpenCourant/OpenCourant, AGPL v3), com
pacotes Linux prontos (`OpenCourant_linux64.zip`, release latest-20261011) que já trazem o
extlib v82, e imagens Docker/Apptainer. É a referência da fase 3 daqui em diante. Nesta sessão
de nuvem o download não passa (GitHub só serve repositórios do dono, e rodar binário baixado é
bloqueado); a fase 3 roda na máquina do dono (ver `docs/PROMPT-CLI.md`).

## 15. Fase 3 — primeira comparação completa (10/11/2026)

Referência: OpenCourant `latest-20261011` (pacote Linux), rodando aqui. Ferramentas:
`fem/radioss.rs` (exporta a MESMA malha e materiais para deck `.rad`), `examples/calib_case.rs`,
`examples/calib_ammo.rs`, `calib/run_calib.py`, `calib/run_ammo.py`, `calib/one.sh`.
Resultados em `calib/ref/esfera_chapa.csv` (80 casos, 12 elementos no diâmetro).

Achados de método (importantes para quem continuar):
- A massa por peça no histórico do Radioss perde os elementos apagados; XMOM/MASS inflava a
  velocidade (chegou a dar 1859 m/s num tiro de 1800). Medir **por nó** (`/TH/NODE`, massas
  nodais exportadas em `.mass`).
- O OpenCourant abortava ("NODAL VELOCITY IS TOO HIGH FOR INTERFACE") em 10 casos oblíquos com
  elementos muito distorcidos. Com `/DT/BRICK/DEL` e `/DT/INTER/DEL` a 20% do passo inicial do
  PenSim (o PenSim apaga a 10%), nenhum aborto, e o resultado varia pouco entre 20% e 30%.
- O atrito (0,1 × 0) quase não muda nada nos dois.

Resultado (80 casos): mesmo desfecho em 63/80; nos 23 casos de perfuração clara, erro médio de
9,6% na velocidade residual (12 dentro de 5%, 14 dentro de 10%). Energia do PenSim < 1,1%.

Desvios que sobram:
1. **Perto do limite balístico a chapa do PenSim cede antes** (Al 10 mm 500–700 m/s, RHA 5 mm
   1200 m/s, RHA 10 mm 1500–1800 m/s): o limite balístico do PenSim sai baixo. Hipóteses a testar,
   uma de cada vez: (a) elemento/hourglass — rodar o OpenCourant com Isolid=1 ou o PenSim com
   `PENSIM_HG` maior; (b) dano/erosão — erosão por distorção do PenSim a 10% vs 20%; (c) EOS/energia.
2. **RHA a 60°**: o PenSim dá 500–600 m/s, o OpenCourant 640–780 (ricochete).
- Munições: o OpenCourant aborta por causa dos elementos minúsculos da ponta da ogiva (raio mínimo
  de ¼ do passo em `revolution_x`). Engrossar a ponta antes de refazer `run_ammo.py`.

## 16. Rotação do projétil

`Config::spin` [rad/s no tempo do modelo]: velocidade inicial ω d × r em torno do eixo de voo;
desliga a simetria. Controle no app (aba Munição, junto da velocidade) em mil rpm reais com botão
"típico" (passo de 30 calibres). Teste único (.50 BMG × 20 mm RHA a 45°, 8 no diâmetro, PenSim):
com 170 mil rpm o projétil perdeu ~14% mais elementos e o desvio lateral trocou de lado. Indício,
não conclusão: repetir com malha fina e vários ângulos, e comparar com o OpenCourant (giro entra
no deck pela velocidade inicial por nó).

## 17. Calibração no PC do dono — baseline (10/11/2026)

Máquina com 36 núcleos (18 para cada solver). OpenCourant `latest-20261011` em
`~/claude/oc_pkg/x/OpenCourant` (SHA-256 do zip conferido). 80 casos, ND=12, 25 min no total
(~19 s por caso), resultado em `calib/ref/baseline_pc.csv`.

- **OpenCourant**: os 80 resultados saem idênticos (<1%) aos de `esfera_chapa.csv`. Determinístico.
- **PenSim**: 51 dos 80 diferem mais de 1% da referência anterior (perto do limite balístico
  até ~30–80 m/s; ex.: Al 10 mm 500 m/s: 97 → 67; RHA 10 mm 1500 m/s: 84 → 8). O PenSim **não é
  reprodutível entre máquinas/número de núcleos** (soma em ordem diferente + erosão caótica).
  Em escala agregada o resultado é o mesmo: com a mesma métrica (perfuração se vx > 30 m/s;
  erro só quando os dois > 100 m/s), referência anterior = 67/80 mesmo desfecho, erro médio 20,0%
  (n=42); baseline = 68/80, 19,8% (n=42). (Esta métrica é mais larga que a da seção 15, por isso
  os números absolutos diferem; o que vale é comparar um arquivo com o outro.)
- Consequência para as tarefas seguintes: uma diferença de poucos % num caso perto do limite
  balístico é ruído do PenSim, não efeito. Os testes de hipótese precisam de vários casos ou de
  fixar `PENSIM_THREADS`.
- Pegadinha: `calib/one.sh` usa `nproc` núcleos no OpenCourant (36) e deu +57 m/s no Al 10 mm
  700 m/s, contra −60 da referência (18 núcleos): o OpenCourant também varia com o número de
  núcleos nesse caso limite.

## 18. Desvio A, hipótese (a): elemento/hourglass (10/11/2026)

Caso: esfera 4340 Ø10 × Al 6061-T6 10 mm, 0°, 700 m/s, ND=12, DTFRAC 0,2. vx residual (m/s):

| Variante | PenSim | OpenCourant |
|---|---|---|
| base (PenSim HG 0,1; OC ISOLID 24) | 404 | −60 |
| PenSim com 6 / 12 / 30 núcleos | 404 / 404 / 404 | −60 |
| PenSim HG 0,05 | 414 | — |
| PenSim HG 0,2 | 381 | — |
| OC ISOLID 1 (1 ponto, hourglass viscoso) | 404 | 302 |
| OC ISOLID 2 | 404 | 291 |
| OC ISOLID 17 | 404 | 454 |
| OC ISOLID 14 | — | inválido (239 ciclos, volume negativo) |

Leitura:
- Mudar o hourglass do PenSim (0,05–0,2) move o resultado ±5%. Não explica 404 × −60.
- Só trocar o elemento no OpenCourant move o resultado de −60 a 454 m/s. Com os elementos de
  1 ponto (ISOLID 1/2), o OpenCourant dá ~300, bem mais perto do PenSim (404) que o ISOLID 24
  da referência. Neste caso limite, **a "referência" depende do elemento**: o ISOLID 24 é o
  mais resistente dos testados.
- O PenSim aqui não variou com o número de núcleos (404,213 com 6, 12 e 30). A diferença para
  a referência antiga (410,4) vem de outra coisa (CPU/compilador); a explicação da seção 17
  ("ordem de soma por núcleos") fica como hipótese NÃO confirmada.
- Falta confirmar nos 80 casos (OpenCourant com ISOLID=1; ver seção seguinte).

Confirmação nos 80 casos (OpenCourant com ISOLID=1, `calib/ref/isolid1.csv`; mesma métrica da seção 17):

| OpenCourant | mesmo desfecho | perfuração clara (n) | erro médio | ≤5% | ≤10% |
|---|---|---|---|---|---|
| ISOLID 24 (referência) | 68/80 | 42 | 19,8% | 16 | 19 |
| ISOLID 1 | 70/80 | 45 | 18,0% | 13 | 20 |

Conclusão de (a): **só explica uma parte**. O elemento resolve o Al 10 mm a 0° em 700 m/s (404 × 302)
e pouco mais; o resultado global quase não muda (+2 desfechos, erro 19,8 → 18,0%). RHA 10 mm 0° a
1800 m/s continua 345 (PenSim) × 51 (OC1), e o RHA a 60° piora (RHA 5 mm 1500 m/s: PenSim 429 ×
OC 768). Ou seja, o desvio B e o limite balístico do RHA não vêm do elemento. Hipótese (a) não é
a causa principal; PenSim fica com o hourglass atual. Próxima: (b) erosão.
