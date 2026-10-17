# Filosofia de design da PawSuite

Este documento vale para todos os apps da suíte (PawCut, PawVoice, PawWrite, PawPaint e os
próximos). Quando um app novo nascer, ou uma tela nova for desenhada, a pergunta é sempre:
"isso parece e se comporta como o resto da suíte?". Se a resposta for não, o app errado é o
que está mudando, não a suíte.

O código que torna isso possível mora em `crates/paw-ui` (tema, ícones, avisos, backups,
helpers de layout) e `crates/paw-audio`. Regra de ouro: **se dois apps precisam da mesma coisa,
ela vai para um crate compartilhado**, nunca é copiada.

## 1. Princípios

1. **Ferramenta, não vitrine.** O app some e o trabalho aparece. Nada de animação decorativa,
   gradiente, sombra pesada ou cor que não signifique algo.
2. **Familiar antes de original.** Cada app imita o programa que ele substitui (PawCut é a aba
   Edit do Resolve, PawVoice é o Audacity, PawPaint é o Krita). Atalhos, nomes e lugares dos
   controles seguem o original. Quem já sabe usar o original não aprende nada de novo.
3. **Escuro e calmo.** Cinzas escuros de baixo contraste, um único laranja de destaque. O
   conteúdo do usuário (vídeo, onda, imagem, texto) é a coisa mais clara da tela.
4. **Previsível.** O mesmo gesto faz a mesma coisa em todos os apps. Nada muda de tamanho,
   de lugar ou de ordem sozinho.
5. **Nunca perder trabalho.** Autosave, recuperação depois de fechar de repente, backups das
   versões anteriores e desfazer em tudo.
6. **Rápido de verdade.** A interface nunca espera por disco, decodificação ou rede. Trabalho
   pesado vai para outra thread, a tela mostra o que já tem e atualiza quando chega.
7. **Em português, direto.** Texto de interface curto, no imperativo ou no substantivo
   ("Salvar", "Remover silêncios"), sem jargão que o original em inglês não use.

## 2. Visual

### Cores (`paw_ui::theme`)

Use sempre as constantes, nunca um `Color32::from_rgb` solto para cor de interface.

| Constante | Uso |
|---|---|
| `BG` | fundo dos painéis |
| `BG_DARK` | fundo de áreas de conteúdo e campos de texto |
| `BAR` / `BAR_LIGHT` | barras de ferramentas, abas, botões em repouso |
| `BORDER` | divisórias de 1 px (quase preto) |
| `TEXT` | texto normal |
| `TEXT_DIM` | rótulos e texto secundário (contraste mínimo de 4,5:1 sobre `BG` e `BAR`) |
| `ACCENT` | seleção, item ativo, foco, ação principal. **Uma cor só.** |
| `PLAYHEAD` | cursor de reprodução |
| `TIMECODE` | números de tempo |
| `OFFLINE` | mídia ou arquivo que sumiu |

Cores de tipo de conteúdo (`CLIP_VIDEO`, `CLIP_AUDIO`, `CLIP_TITLE`...) são dados, não
decoração: a mesma cor significa a mesma coisa em todo app.

Laranja (`ACCENT`) indica "isto está ativo ou selecionado". Vermelho indica erro ou perigo.
Nenhuma outra cor carrega significado.

### Tipografia

- Noto Sans (ou DejaVu Sans se faltar). Corpo e botões em 12,5 px, texto pequeno 10,5 px,
  títulos 16 px, tempo e números alinhados em monoespaçada.
- Rótulos de linha em `TEXT_DIM`, valores em `TEXT`. Títulos de seção em branco.
- Nunca use emoji nem símbolo de fonte como ícone (🔒, ↺, ↔): a fonte do sistema pode não ter,
  e cada um tem um traço diferente. Use `paw_ui::icons`.

### Grade

Tudo é múltiplo de 4 px.

| Constante | Valor | Uso |
|---|---|---|
| `TOOLBAR_H` | 28 | altura de toda barra de ferramentas |
| `ROW_H` | 24 | altura de toda linha de propriedade |
| `PAD` | 8 | margem interna padrão |
| `ICON` | 16 | tamanho de ícone em botão |
| `LABEL_W` | 104 | coluna dos rótulos no Inspector |
| `VALUE_W` | 84 | coluna dos valores |
| `NUM_W` | 64 | campo numérico solto (fora de uma linha de propriedades) |

Cantos com raio 3 (widgets) e 4 (janelas). Bordas de 1 px em `BORDER`. Espaço entre itens 8 x 4.

### Ícones e botões

- **Um estilo só** (`paw_ui::icons`): contorno de traço uniforme (2/24 do tamanho), desenhado
  num grid de 24 × 24 com margem de 2, cantos arredondados e pontas redondas, como os ícones de
  linha do Resolve e do Krita. Só o transporte (tocar, pausar, parar, gravar) é cheio. Um ícone
  novo entra no `enum Icon` e em `Icon::ALL`; o teste confere que ele não sai do quadrado e o
  exemplo `cargo run -p paw-ui --example icon_sheet` mostra a folha inteira para comparar.
- Ferramentas e ações de todos os apps usam os mesmos desenhos (a borracha do PawPaint é a mesma
  da roda de atalhos; o salvar é o mesmo nos quatro).
- Botão de ícone: `paw_ui::icons::button(ui, icon, size, ligado, dica)`. **Todo** botão sem
  texto tem dica (tooltip), e a dica mostra o atalho: "Cortar no cursor (Ctrl+B)".
- Estado ligado = fundo mais claro + ícone em `ACCENT`. Desligado = `TEXT_DIM`. Passar o mouse
  clareia. Desativado não responde e fica apagado.

### Sliders

- Sempre `paw_ui::Slider` (nunca o `egui::Slider`, que tem botão retangular e campo que muda de
  largura): trilho fino de 4 px, parte preenchida em `ACCENT`, botão redondo claro e o número num
  campo de tamanho fixo (`slider::VALUE_W`) à direita.
- Intervalo que passa pelo zero (ganho, pan, brilho, matiz) preenche **a partir do zero**, com
  uma marquinha no meio. Tamanho de pincel e zoom usam escala logarítmica.
- Com `.text("Rótulo")` o rótulo vai à **esquerda**, numa coluna fixa (`slider::LABEL_W`): vários
  sliders seguidos ficam com os trilhos alinhados. Numa `Grid` o slider inteiro é uma célula.
- No Inspector do PawCut o slider ocupa a coluna do valor e o número fica sempre na mesma posição;
  um parâmetro sem slider útil (posição) deixa o espaço vazio para os números continuarem
  alinhados.

## 3. Estrutura da tela

- **Painéis acopláveis** (`egui_dock`): o usuário arruma, fecha, divide e o layout fica salvo.
  Sempre existe um jeito de reabrir um painel fechado (menu Workspace ou Configurações) e um
  "Resetar layout".
- **Barra de ferramentas** de cada painel: `paw_ui::theme::toolbar`. Mesma altura, mesma margem,
  alinhamento vertical central em todos.
- **Seções recolhíveis**: `theme::section`, com setinha. O estado aberto ou fechado é lembrado.
- **Propriedades em grade fixa**: `theme::row(rótulo, valor, extras)`. Rótulo alinhado à direita,
  valor, e até dois botões no fim (keyframe e voltar ao padrão, com os ícones da suíte) **sempre nas mesmas colunas**.
  Se uma linha não usa o segundo botão, reserve o espaço (`ui.add_space(22.0)`) para o botão de voltar não
  andar para o lado.
- **Barra de estado** embaixo: informação que muda sempre (tempo, contagens, zoom), nunca
  botão perigoso.
- **Diálogos** passam por `paw_ui::dialog`: `window` para opções (Exportar, Novo, Filtro…) e
  `modal` para decisão que bloqueia (sair sem salvar, recuperação, efeito rodando). Mesma
  posição (centralizado na horizontal, perto do topo), largura fixa que encolhe em janela
  estreita, sem seta de recolher e sem "×": fecha por Cancelar/Fechar ou Esc. Conteúdo alto vai
  em `dialog::scroll` e o pé fica à vista. Confirmação de perigo: Salvar, Não salvar, Cancelar.
  Janelas de ferramenta que ficam abertas enquanto se trabalha (Histórico) são a exceção e
  mantêm o "×".
- **Pé dos diálogos** (`theme::dialog_actions`): linha fina separando, botões alinhados à
  direita, a ação principal na ponta em laranja (`theme::primary_button`) e Cancelar à esquerda
  dela.
- **Caixa de ferramentas** (PawPaint): grade por grupos (pintura, seleção, mover, formas) que
  reflui conforme a largura do painel até virar uma coluna só, como a do Photoshop; o botão de
  setinhas no topo fixa em uma coluna. Barra de cima larga demais recolhe o resto num "⋯".
- **Menus** da barra de cima rolam quando não cabem na tela (`top_menu`), nunca cortam itens.
- **Avisos** vão para `paw_ui::toasts` (canto de baixo, somem sozinhos, erro em vermelho e fica
  mais tempo). Diálogo bloqueante só para o que exige decisão.

## 4. Campos de entrada (a regra que mais quebra)

Um campo **nunca** pode mudar de tamanho enquanto o usuário usa ele, nem empurrar o que está
em volta, nem vazar para fora da linha.

1. **Largura fixa.** O `DragValue` do egui cresce e encolhe com os dígitos e o sufixo. Dentro
   de `theme::row` isso já é resolvido (`VALUE_W`). Fora dela, embrulhe:
   ```rust
   theme::fixed_width(ui, theme::NUM_W, |ui| {
       ui.add(DragValue::new(&mut w).range(16..=8192));
   });
   ```
2. **Nunca passe de uma largura fixa maior que o espaço.** Para `TextEdit` e `ComboBox`, use
   `theme::fit_width(ui, 140.0)`: ele usa a largura desejada, mas encolhe em painel estreito.
3. **Altura fixa em texto de várias linhas.** Um `TextEdit::multiline` cresce a cada linha e
   empurra o painel. Ponha em um `ScrollArea` com `max_height`.
4. **Velocidade do arrasto em unidades da tela.** `DragValue::speed` é "quanto muda por pixel",
   na unidade que o usuário vê (%, px, °, dB). Para porcentagem, 0,5 por pixel. Nunca
   multiplique pela escala interna.
5. **Sempre com `range`.** Todo campo numérico tem mínimo e máximo, e o modelo (core) valida de
   novo: a interface ajuda, o core protege.
6. **Digitar e arrastar dão o mesmo resultado**, com sufixo (`" %"`, `" dB"`, `" px"`, `" fr"`)
   e o mesmo número de casas decimais. O egui deduz as casas decimais de `speed`: campos
   parecidos usam o mesmo `speed`, senão um mostra "100 %" e o vizinho "100.0 %".
7. **Um passo de desfazer por gesto.** Arrastar um valor aplica a edição com
   `apply_coalescing(chave)`; soltar o mouse, Enter ou Tab fecham o passo.
8. **Campo de texto foca = atalhos calam.** Com um campo de texto em foco, nenhum atalho de
   tecla simples (Espaço, letras, setas) pode disparar. O app consulta `ctx.text_edit_focused()`.
9. **Valor misto.** Com vários itens selecionados e valores diferentes, mostre o do primeiro e
   a etiqueta "vários". Mudar o valor muda todos.
10. **Enter confirma, Esc cancela**, em todo campo que valida o que foi digitado (timecode,
    duração, nome) e em todo diálogo de efeito. Exceções por segurança: Enter nunca confirma
    "Sair sem salvar" nem uma recuperação, e Esc nunca descarta uma recuperação.
11. **Só aplica o que foi editado.** Um campo que mostra o valor arredondado (milissegundos,
    uma casa decimal) não pode gravar o valor arredondado só porque ganhou e perdeu o foco.

### Painéis e janelas pequenos

O app abre bem em 960 x 600 e continua usável.

- Painéis têm largura mínima (`theme::PROPS_MIN_W`, 260 px no PawCut). Abaixo disso o Inspector
  cortaria campos.
- A coluna dos rótulos (`theme::row`) encolhe antes do campo de valor, e o rótulo é recortado na
  própria coluna.
- Menus e listas mais altos que a tela rolam (`ScrollArea` com `max_height`), em vez de cortar
  os últimos itens.
- Janelas abrem centralizadas na horizontal e perto do topo (o topo fica fixo se a altura
  mudar). Se não couberem, **só as opções rolam** (`ScrollArea` em volta delas, com
  `max_height` calculado a partir da altura da tela): o resumo e os botões de ação ficam sempre
  à vista. Não use `Window::vscroll(true)`: ele limita a altura da janela e esconde o botão
  principal embaixo da dobra.
- Nunca use `f32::clamp(min, max)` com limites que dependem do tamanho de algo na tela: se
  `min > max` o app cai. Use um clamp seguro (`clamp_in`) que fica no meio quando o intervalo
  se inverte.
- **Barra de ferramentas larga demais** não vaza nem esconde coisa sem aviso: o que não cabe vai
  para um menu "⋯" no fim da barra (PawPaint: `ui.available_width() < 1180.0` recolhe Espelho e
  Borracha). Barra de busca e de status quebram a linha em vez de estourar a janela.
- **Painéis acoplados (docks) rolam**, e sliders dentro deles medem a largura que sobrou
  (`slider_width`), em vez de usar os 100 px do egui, que cortam o valor em painel estreito.
  Miniaturas e vistas dentro do painel se desenham dentro do próprio retângulo (`painter_at`).
- **Diálogo pequeno:** campos de tamanho em px e sliders com largura fixa (`NUM_W`) e no máximo
  duas casas decimais, e um teste que abre o diálogo com os valores mais curtos e os mais longos
  e confere que a janela não muda de largura.

## 5. Interação

### Roda de atalhos (botão do meio)

Igual nos quatro apps (`paw_ui::radial`), como a roda de comunicação dos jogos e o "pie menu" do
Blender:

- **Segure** o botão do meio em qualquer lugar, **arraste** na direção da ação e **solte**. Soltar
  no centro (ou numa fatia vazia) cancela.
- Um **toque rápido** deixa a roda aberta: clique numa fatia para escolher, no miolo para
  personalizar, fora ou Esc para fechar.
- Oito posições, a partir de cima no sentido horário. O miolo mostra o nome completo da ação
  apontada. Cada app tem um catálogo (id, nome, ícone) e a janela "Roda de atalhos…" (no menu
  Configurações, Ferramentas ou Workspace) escolhe a ação de cada posição; fica salvo em
  `radial.json` na pasta de configuração.
- O botão do meio é **só** da roda: nenhum painel arrasta nada com ele (use
  `dragged_by(PointerButton::Primary)`). Arrastar a tela do PawPaint é Espaço + arrastar; o
  visualizador ampliado do PawCut move com Alt + arrastar.
- Com a roda ou a personalização abertas, os atalhos de teclado do app calam (`Radial::is_open`).

### Geral

- **Atalhos editáveis** e iguais entre apps: Espaço toca e pausa, Ctrl+Z / Ctrl+Shift+Z,
  Ctrl+S / Ctrl+Shift+S, Ctrl+N, Ctrl+O, Ctrl+C / Ctrl+X / Ctrl+V, Delete, Ctrl+roda dá zoom, a
  roda rola. Um atalho novo nunca rouba o de outro app; se o original (Resolve, Audacity, Krita)
  usa uma tecla, a nossa usa a mesma.
- **Paleta de comandos** (Ctrl+K) lista toda ação com o atalho ao lado.
- **Atalhos aparecem como o usuário os digita**: `Shift+[`, não `Shift+OpenBracket`; `1`, não
  `Num1` (`key_label` no keymap).
- **Botão direito** abre o que se faz com aquilo (menu de contexto), nunca é o único caminho.
- **Arrastar e soltar** arquivos na janela importa/abre.
- **Cursor do mouse** diz o que vai acontecer (redimensionar, texto, mão).
- **Botão de barra é ícone, não texto.** Em toolbar e barra de transporte, o que cabe num ícone
  (`paw_ui::icons::button`) é ícone, com a explicação na dica. Um ícone ao lado de um botão de
  texto que faz a mesma coisa é bug: vira um só, o ícone, e o clique faz a ação (foi o caso da
  engrenagem solta ao lado de "Configuração de áudio"). Texto só quando não existe ícone claro
  ou a ação precisa de uma palavra para não ser confundida.
- **Feedback imediato.** Todo clique tem resposta na mesma tela (< 100 ms). Se a operação é
  mais lenta, mostre progresso e permita cancelar.

## 6. Áudio

Regras para qualquer app que toque ou grave som (`paw-audio` e o player do PawCut):

1. **Editar durante o play não reinicia o som, e vale na hora.** Ganho, pan, mudo, solo e
   envelope mexidos enquanto toca são ouvidos no bloco seguinte, como no Audacity e no Resolve.
   O player não toca um buffer pré-mixado: ele pede o som aos poucos a uma fonte
   (`paw_audio::mix::MixSource`), e uma edição só troca a fonte (`Player::update_live`).
   Nunca esvazie a fila da placa (`flush`) por causa de uma edição: isso repete trechos e
   picota o áudio a cada ajuste (era o bug do volume no PawCut). `flush` só em play, seek,
   scrub e stop. Pré-mixar o trecho inteiro antes do play também é proibido: 5 minutos de 4
   faixas levavam ~0,6 s na thread da tela (30 minutos, ~3,5 s de janela travada); um bloco
   de 10 ms ao vivo leva 0,02 ms.
2. **Sem degrau.** Quando um parâmetro muda no meio da reprodução, passe do som antigo para o
   novo com um crossfade curto (um bloco de 10 a 20 ms), senão dá estalo. Use
   `paw_audio::mix::crossfade`, que PawCut e PawVoice compartilham.
3. **Buffer curto.** ~60 a 100 ms de fila na placa (`BufferAttr`; 60 ms no `paw-audio`, 100 ms
   no PawCut, que mixa mais coisa por bloco), para que o que o usuário fez seja ouvido quase na
   hora. O padrão do PulseAudio (~250 ms) parece lento.
4. **O áudio é o relógio.** O cursor de reprodução vem da posição que já saiu da caixa de som
   (posição escrita menos a latência), não de um cronômetro.
5. **A thread de áudio nunca espera pela interface, nem decodifica.** Estado compartilhado por
   atômicos ou `Mutex` com trava curta. A decodificação roda numa thread à parte
   (`AudioLoader`) e entrega o PCM pronto; o laço de áudio só mixa o que já está na memória
   (`mix_audio_resident`). Um clipe novo fica mudo por um instante em vez de travar o som dos
   outros. Se algo sair da memória (orçamento de cache), é pedido de novo sozinho.
6. **Medidores** sobem na hora e descem devagar (x0,85 por bloco). Pico em 0..1, escala em dB.
7. **Loudness** em LUFS (EBU R128), alvo de −14 para YouTube, e nunca passa de −1 dB de pico.
8. **Exportar em etapas visíveis.** A mixagem do áudio acontece antes dos frames e pode demorar:
   o progresso informa a etapa ("Preparando o áudio…", depois "n / total frames"), e o arquivo
   temporário do áudio é de 16 bits (metade do disco do float; o destino é AAC, MP3 ou WAV de
   16 bits).
9. **Efeito demorado não congela a janela.** Efeito sobre mais de ~1 minuto de áudio roda numa
   thread (`Editor::begin_process`, o trabalho em `ProcessJob::run`, depois `finish_process`)
   com um aviso "Aplicando…" que bloqueia a tela; se a sessão mudou no meio, o resultado é
   recusado (`Stale`). Referência (release, 10 min de áudio mono): de-esser 1,2 s, redução de
   ruído 1,1 s, compressor 0,8 s (`cargo run --release -p paw-audio --example bench_dsp`).
10. **Compatibilidade com versões do ffmpeg.** Decoders do ffmpeg 6/7 podem não preencher o
   layout de canais (WAV de 16 bits). Sempre tenha o reserva pela contagem de canais e teste
   com arquivos gerados pelo `ffmpeg` de linha de comando em `pcm_s16le`, `pcm_s32le` e
   `pcm_f32le`, mono e estéreo.

### Como provar que o áudio não picota

Sem placa de som, dá para medir. Suba um PulseAudio com um sink nulo
(`pactl load-module module-null-sink sink_name=test`), toque um seno de 440 Hz no app e grave
`test.monitor` com `parec` enquanto arrasta o volume. Num seno puro os cruzamentos por zero
ficam a ~109 amostras (48 kHz); intervalos que fogem mais de 10 % disso são trechos repetidos
ou degraus. Antes do conserto do PawCut eram 28 em ~3000 (um a cada ~0,1 s de arrasto); depois,
zero.

## 7. Arquivos e segurança do trabalho

- Um formato de projeto por app, em texto (JSON) ou zip simples, com `version` para migrar.
- **Autosave** periódico em `~/.local/share/<app>/autosave/` e oferta de **recuperar** ao
  abrir depois de um fechamento brusco.
- **Backups**: ao salvar por cima, a versão anterior vai para `paw_ui::backups` (10 versões).
- Configuração em `~/.config/<app>/` (respeitando `XDG_CONFIG_HOME`), cache em
  `~/.cache/<app>/` (pode apagar à vontade).
- **Fechar com alterações** pergunta (Salvar, Não salvar, Cancelar). Fechar durante uma
  exportação pergunta também.
- **Autosave nunca trava a pintura nem a digitação.** Em arquivo grande o salvamento leva
  segundos: o app tira uma cópia barata do documento (PawPaint: os blocos de pixels são
  compartilhados, ~2 ms) e salva numa thread, num arquivo temporário que só depois vira o
  definitivo. Se o salvamento anterior ainda roda, tenta de novo em 10 s. Um documento fechado
  durante o salvamento não pode ter o arquivo de recuperação recriado (conjunto de fechados).
- **Salvar manual** em arquivo grande também usa todos os núcleos (uma camada por thread) e não
  recomprime PNG dentro do zip.
- **Qualquer caminho que troca o documento** (abrir, recentes, arrastar arquivo, Ctrl+N) pergunta
  antes se há mudanças, e "Salvar" continua o que o usuário queria depois de salvar
  (`after_save`).
- Arquivo que sumiu aparece como offline (vermelho, `OFFLINE`) com Reconectar, nunca derruba o app.
- Erros de arquivo e de mídia viram aviso legível, nunca `panic`. `unwrap` em dado do usuário
  é bug.

## 8. Desempenho

- A thread da interface só desenha e reage. Decodificar, renderizar, calcular picos,
  exportar e analisar vão para threads com pedidos que se substituem (só o mais novo importa).
- Caches com limite (LRU) e chaves que incluem tudo que muda o resultado (arquivo, velocidade,
  tamanho).
- Estruturas imutáveis compartilhadas (`Arc<Project>`) entre interface, preview e áudio, em vez
  de copiar.
- Pedir repaint só quando algo muda ou algo está tocando. Parado, o app fica em 0 % de CPU.
- Medir antes de otimizar, e deixar um teste ou exemplo (`examples/`) que reproduz o gargalo
  (`bench_dsp`, `bench_mix`, `bench_text`, `bench_save`, `bench_ops`). Meça com a máquina
  quieta: dois benchmarks ao mesmo tempo, ou um fuzz rodando, dobram os números.
- **Algoritmo linear em texto e em amostras.** Substituir tudo num texto grande tem de ser uma
  passada só (`replace_all` do PawWrite), nunca "achar, substituir e começar de novo".
- **Imagem grande (12 megapixels) é a regra, não a exceção.** Filtro, preenchimento, achatar e
  montar a textura da tela dividem as linhas entre os núcleos (`pawpaint_core::par`: cada thread
  recebe um pedaço do vetor, sem trava e sem `unsafe`); varrem a memória em ordem (um desfoque
  que andava coluna por coluna levava 8 s, por linhas leva 0,5 s); não guardam cópia do original
  (relêem dos blocos); e só escrevem os blocos que mudaram, para o desfazer continuar
  compartilhando o resto.
- **Desfazer com limite de memória, não só de passos.** Guardar o documento inteiro a cada passo
  é de graça enquanto os blocos são compartilhados, mas 60 filtros de tela cheia numa foto de
  12 megapixels são gigabytes. O histórico soma a memória dos blocos que só ele retém
  (`history_bytes`) e esquece os passos mais antigos acima de 1,5 GB, sempre deixando alguns.
- **Prévia ao vivo adapta ao custo.** Se a última prévia demorou mais que ~60 ms, o filtro só é
  refeito quando o slider é solto (`preview_action`); a primeira prévia sempre aparece.
- **Cache pela identidade do dado, não pela revisão do documento.** O contorno da seleção
  dependia da revisão e era recalculado a cada pincelada; agora só muda quando a máscara muda
  (`Selection::same_mask`).
- **Laços de desenho com entrada do usuário** (carimbos ao longo de um traço) validam o número
  antes: NaN, infinito e coordenadas absurdas são ignorados, e um salto enorme vai direto ao
  destino em vez de carimbar milhões de vezes.

## 9. Qualidade

- `cargo clippy --workspace --all-targets -- -D warnings` e `cargo fmt` limpos antes de commitar.
- Lógica de edição no `*-core`, sem egui, testada por unidade e por propriedade.
- A interface tem testes com cliques e arrastos simulados (`egui_kittest`): um bug de
  interface corrigido ganha um teste.
- `scripts/ui-fuzz.sh <app>` abre o app num display virtual e faz cliques, arrastos e atalhos
  aleatórios. Todo crash que ele achar vira um teste que reproduz (e o teste tem de falhar sem
  o conserto: confira revertendo o conserto).
- **Ver o app de verdade**, sem monitor: `Xvfb :99 -screen 0 1600x950x24`, o app com
  `DISPLAY=:99`, `xdotool` para clicar, arrastar e apertar teclas (`xdotool windowfocus` antes:
  sem gerenciador de janelas as teclas não chegam) e `import -window root foto.png` para a
  captura. Pacotes: `xvfb xdotool imagemagick libxkbcommon-x11-0 libgl1-mesa-dri`.
  Faça isso também em 960 x 600 e com os painéis estreitos: foi assim que apareceram campos
  cortados, menus sem fim e janelas altas demais.
- **Entradas absurdas:** cada core tem um teste que chama toda operação com NaN, infinito,
  números enormes, negativos e tamanhos zero (`extreme_inputs_never_panic` no PawPaint). Ele
  achou um laço infinito no pincel; rode com `timeout` se mexer nele.
- **Otimização com resumo:** antes de reescrever um laço quente, grave um resumo (hash) da saída
  da versão antiga num teste (`stamp_output_digest_is_stable`) e prove que a nova dá o mesmo.
- Mensagem de commit: `tipo(app): o que mudou` (`feat`, `fix`, `perf`, `polish`).

## 10. Erros que já aconteceram (e a regra que saiu deles)

| O que aconteceu | Regra |
|---|---|
| Ajustar o volume durante o play picotava o som: cada edição esvaziava o buffer e recomeçava de uma posição atrasada | 6.1 e 6.2: editar não reinicia, e o som novo entra em crossfade |
| Zoom e Opacidade mudavam 50 % por pixel arrastado | 4.4: `speed` na unidade da tela |
| Inspector estreito cortava Fonte, Tamanho, Cor e o botão Normalizar | painel com largura mínima, rótulo cede antes do campo |
| Clipe de 1 frame com a timeline afastada derrubava o app (`clamp` com min > max) | clamp seguro para tudo que depende de tamanho na tela |
| Menu Editar sem os últimos itens em janela de 600 px | menus rolam |
| Deliver com o botão Exportar fora da tela | só as opções rolam; ação principal sempre à vista |
| WAV de 16 bits não abria no ffmpeg 6/7 (layout de canais vazio) | 6.10: reserva pela contagem de canais |
| Clipe desligado com arquivo sumido bloqueava a exportação | a checagem de mídia offline segue as mesmas regras do renderizador |
| Atalhos escritos como `Shift+OpenBracket` | rótulos de tecla legíveis |
| PawVoice: Ganho, Pan e envelope só valiam no próximo play, e apertar play numa sessão longa travava a janela | 6.1: mixagem ao vivo por `MixSource` |
| PawVoice: aplicar efeito em narração de vários minutos congelava a janela | 6.9: efeito em outra thread |
| PawVoice: confirmar o campo de seleção sem digitar arredondava a seleção para o milissegundo | campo de texto só aplica o que foi editado |
| PawVoice: abrir outro projeto (Ctrl+O, recentes, arrastar arquivo) descartava as mudanças sem perguntar | 7: qualquer caminho que troca o projeto pergunta antes se há mudanças (`ConfirmDiscard`) |
| Trocar a saída de áudio deixava a thread do player antigo rodando para sempre | todo `Player` termina a thread quando é descartado |
| PawVoice: Esc em diálogo, renomear faixa e campo de seleção não cancelava | 4.10 vale para todo campo e diálogo; Esc nunca descarta trabalho (recuperação) |
| PawWrite: "Substituir tudo" em roteiro grande demorava minutos (achava e recomeçava a busca a cada troca) | 8: passada única, com exemplo `bench_text` |
| PawWrite: arrastar um arquivo para a janela descartava o roteiro sem perguntar | 7: todo caminho que troca o documento pergunta (`UnsavedDrop`, `after_save`) |
| PawWrite: barra de busca estourava a janela estreita | 4: barras quebram a linha |
| PawPaint: salvar ou autosave de foto grande congelava a janela por ~10 s | 7: cópia barata do documento, salvamento em thread, PNGs em paralelo |
| PawPaint: desfoque numa foto de 12 MP levava 8 s e a prévia refazia a cada pixel do slider | 8: varrer a memória em ordem, dividir entre núcleos, prévia lenta só ao soltar |
| PawPaint: 60 passos de desfazer em foto grande podiam guardar gigabytes de pixels | 8: o histórico também tem limite de memória |
| PawPaint: o contorno da seleção era recalculado a cada pincelada | 8: cache pela identidade do dado |
| PawPaint: o pincel fazia duas buscas em mapa por pixel e clonava o pincel a cada carimbo | 8: trabalho por bloco; resumo da saída garante que o desenho não mudou |
| PawPaint: seleção retangular arrastada toda para fora da imagem derrubava o app (fuzz) | limites nos dois lados, nunca só em um |
| PawPaint: um ponto do pincel em 1e30 ou NaN travava o app em laço infinito | 8: validar número antes de laço de desenho |
| PawPaint: sliders e dock de pincéis cortados em painel estreito | 4: docks rolam, slider mede o espaço que sobrou |
| Sliders com botão retangular e número que mudava de largura, rótulos depois do valor, ícones de estilos diferentes (e emoji) | 2: `paw_ui::Slider` e `paw_ui::icons` em todos os apps |
| PawPaint: a caixa de ferramentas não encolhia abaixo de 175 px (mínimo padrão do `egui_dock`) | `separator.extra` do dock no tamanho que o painel aguenta de verdade |
| Um slider dentro de uma `Grid` virava duas colunas (trilho e campo), empurrando o valor para fora do docker | widget composto desenha numa `horizontal` interna |
| PawVoice: arrastar com o botão do meio selecionava áudio por baixo da roda | 5: painéis só respondem ao botão esquerdo |

## 11. Checklist para uma tela nova

- [ ] Só usa cores e medidas de `paw_ui::theme`.
- [ ] Barras e linhas usam `toolbar`, `section` e `row`.
- [ ] Todo botão de ícone tem dica com o atalho.
- [ ] Nenhum botão de texto numa barra onde um ícone resolve; nenhum ícone decorativo que não clica.
- [ ] Todo campo tem largura fixa (ou `fit_width`), `range` e sufixo.
- [ ] Nenhum campo muda de tamanho ao arrastar ou digitar, nem em painel estreito.
- [ ] Com um campo de texto em foco, os atalhos não disparam.
- [ ] Um gesto = um passo de desfazer.
- [ ] Trabalho pesado fora da thread da interface.
- [ ] Funciona com o painel pequeno, a janela em 960 x 600 e o painel maximizado.
- [ ] Menu ou lista alta rola; janela alta rola só as opções; o botão principal fica à vista.
- [ ] Nenhum `clamp(min, max)` com limites que dependem do tamanho da tela.
- [ ] Nada decodifica, lê disco grande ou espera rede na thread de interface ou na de áudio.
- [ ] Operação longa mostra a etapa e deixa cancelar (ou roda em outra thread com aviso).
- [ ] Mexer num parâmetro durante o play vale na hora, sem reiniciar o som.
- [ ] Passou pelo `scripts/ui-fuzz.sh` sem cair, em mais de uma semente.
- [ ] Funciona com um arquivo grande (foto de 12 MP, texto de 1 MB, narração de 30 min) sem congelar.
- [ ] Toda operação que recebe número do usuário sobrevive a NaN, infinito, enorme e zero.
- [ ] Sliders são `paw_ui::Slider`; ícones são `paw_ui::icons` (nada de emoji).
- [ ] Ações novas entram no catálogo da roda de atalhos do app.
- [ ] Diálogos terminam com `dialog_actions` e a ação principal em `primary_button`.
- [ ] Texto em português, curto, sem traço longo.
