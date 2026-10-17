# PawSuite

Apps próprios, em Rust, para criar vídeos do YouTube sem depender de programas de outras
empresas. Todos usam a mesma interface escura (no estilo do DaVinci Resolve) e as mesmas
bibliotecas:

| App | Para quê | Estado |
|---|---|---|
| **PawCut** | editor de vídeo | pronto |
| **PawVoice** | gravar e editar narração | primeira versão |
| **PawWrite** | roteiros (no lugar do Word) | primeira versão |
| **PawPaint** | imagens e pintura (no lugar do Krita) | primeira versão |

```bash
scripts/install.sh            # compila e instala todos (menu de aplicativos + ~/.local/bin)
scripts/install.sh pawvoice   # só um (pawcut, pawvoice, pawwrite, pawpaint)
```

Como os apps se parecem e se comportam está em [`docs/DESIGN.md`](docs/DESIGN.md).

**Roda de atalhos (nos 4 apps):** segure o botão do meio do mouse, arraste até a ação e solte
(como a roda de comunicação dos jogos). Um toque rápido deixa a roda aberta para clicar; o centro
dela abre a personalização das oito posições (também no menu: "Roda de atalhos…").

Bibliotecas compartilhadas em `crates/`:
- `paw-ui`: tema, ícones, avisos e backups;
- `paw-audio`: loudness, filtros, gate, compressor, tocar e gravar;
- `paw-media`: ffmpeg.

## PawVoice

Gravador e editor de voz com a interface e o jeito de usar do Audacity.

- **Tela:**
  - transporte (Pausar, Tocar, Parar, Início, Fim, Gravar, Loop);
  - ferramentas Seleção (F1) e Envelope (F2), botões de edição e **Configuração de áudio**;
  - medidores **Mic** e **Saída** com L/R (clique no medidor para monitorar o microfone sem gravar);
  - régua, faixas mono ou **estéreo** (L em cima, R embaixo) e barra de seleção.
- **Configuração de áudio:** escolha o microfone e a saída, e grave em mono ou estéreo. O padrão é
  o que o microfone tiver; um microfone binaural/estéreo grava os dois lados. Fica salva entre as sessões.
- **Gravar:**
  - R grava na faixa selecionada a partir do cursor; Shift+R grava numa faixa nova;
  - as outras faixas tocam enquanto grava (overdub);
  - começa na hora e não perde o fim; se o app fechar no meio, ele oferece recuperar.
- **Editar como no Audacity:**
  - arrastar seleciona (na vertical pega várias faixas); Z ajusta a seleção ao cruzamento por zero;
  - clique no cabeçalho do clipe seleciona e Ctrl+clique soma outros; arrastar move todos juntos
    (Time Shift), e um clipe sozinho pode ir para outra faixa;
  - mexer em Ganho/Pan/Mudo/Solo de uma faixa selecionada vale para todas as selecionadas, e tudo (inclusive o
    envelope) vale **na hora, enquanto toca**;
  - Ctrl+X, Ctrl+C e Ctrl+V; Delete; Ctrl+D duplica; Ctrl+L silencia; Ctrl+T apara;
    Ctrl+Alt+I separa em nova faixa;
  - Ctrl+1 e Ctrl+3 dão zoom, Ctrl+F cabe tudo, Ctrl+E dá zoom na seleção;
  - Espaço toca/para, P pausa, Shift+Espaço repete.
- **Envelope (F2):**
  - clique cria um ponto e arrastar muda o volume ao longo do tempo (ex.: abaixar a música
    debaixo da voz);
  - botão direito, ou arrastar para fora, apaga o ponto.
- **Efeitos** (na seleção; sem seleção, em tudo):
  - **Melhorar voz (1 clique)**;
  - **Redução de ruído** com perfil (como no Audacity: pegue o perfil num trecho só de chiado e aplique);
  - normalizar loudness (LUFS) ou pico, amplificar, compressor, limitador, fades;
  - noise gate, passa-altas, graves e agudos, **de-esser** (tira o chiado do S), truncar silêncio e inverter;
  - Arquivo → *Exportar para vídeo* já ajusta o loudness para o alvo (−14 LUFS no YouTube).
  - efeitos em trechos longos rodam em segundo plano (aparece "Aplicando…"), a janela não congela;
  - nos diálogos de efeito, Esc cancela e Enter aplica.
- **Mais efeitos:** **equalizador** paramétrico com curva e presets (voz clara, rádio, telefone, zumbido 60 Hz),
  notch, reverb, tremolo, phaser, wah-wah, **velocidade**, **tempo** (muda a duração sem mudar a voz) e **tom**
  (sem mudar a duração), reparar áudio estourado, apagar uma faixa de frequências, fade ajustável, repetir, tirar a
  voz do centro (karaokê), **auto-duck** (a música abaixa quando a voz fala) e **crossfade** entre duas faixas
  selecionadas. **Ctrl+R** repete o último efeito, com os mesmos valores, na seleção de agora.
- **Análise:** rotular as falas (um rótulo por trecho com som), marcar onde o áudio estourou e **transcrever em
  rótulos** (precisa do `whisper` instalado: `pip install openai-whisper` ou o whisper.cpp com um modelo ggml).
- **Gerar:** Faixas → Gerar tom, ruído (branco, rosa, marrom) ou metrônomo.
- **Região de loop:** Shift + arrastar na régua (ou Transporte → Região de loop) e ligar o loop toca só ela.
- **Roteiro e teleprompter** (Ver → Roteiro): a letra grande rola sozinha enquanto você grava.
- **Arquivos:**
  - o projeto fica em `nome.pawvoice`, com os áudios em `nome.pawvoice-audio/` (WAV, mono ou estéreo);
  - exporta a mixagem ou só a seleção em WAV/MP3, ou cada faixa separada.

## PawWrite

Editor de roteiros para vídeo: texto limpo e o tempo de fala sempre à vista.

- **Escrever:**
  - `# Título` abre uma seção (`##` para subseções) e aparece na **Estrutura**, à esquerda, com o tempo de cada parte;
  - `[B-roll: mesa]` é nota de edição: fica em azul e **não conta** no tempo;
  - `// lembrete` é comentário só seu;
  - o resto é a fala. Embaixo: palavras, tempo de fala e o seu **ritmo** (palavras por minuto, ajustável).
- **Editar:** Ctrl+F localiza (F3 próximo), Ctrl+H substitui, Ctrl++ / Ctrl+- mudam o tamanho da letra,
  Ctrl+\ mostra ou esconde a estrutura. Clicar numa seção da estrutura leva o texto até ela.
- **Arquivos:** roteiro em Markdown (`.md`) ou texto. Salvar guarda as 10 versões anteriores; se o app
  fechar sem salvar, ele oferece recuperar o texto.
- **Com o PawVoice:** Arquivo → *Abrir no PawVoice* manda o roteiro para o teleprompter. O PawVoice
  mostra só a fala (sem títulos, notas e comentários). Também dá para exportar ou copiar só a fala.
  Arquivo → *Exportar legendas (.srt) pelo ritmo* gera legendas da fala com o tempo calculado, para importar
  no PawCut e ajustar. Rótulos do PawVoice viram capítulos do YouTube.
- **Para o YouTube:** Arquivo → *Copiar capítulos do YouTube* monta a lista (`00:00 Introdução`, `00:45 …`) a partir dos
  títulos e do tempo de fala, e avisa se falta algo (pelo menos 3 capítulos de 10 s). Ver → *Análise do texto* mostra as
  muletas (então, tipo, né…), as palavras que mais se repetem, as frases longas demais e o que é dito nos primeiros 30 s.
- **Mais:** mover seções pela estrutura (botão direito), meta de duração do vídeo na barra de baixo e modo leitura
  (F5) com a fala em letra grande rolando sozinha.
- **Seguro com o seu texto:** arrastar um arquivo para a janela, abrir outro ou criar um novo pergunta antes de
  descartar mudanças (e "Salvar" continua a ação depois de salvar); Esc fecha diálogos e a barra de busca.
  Substituir tudo é uma passada só, mesmo em roteiros enormes.

## PawPaint

Editor de imagens no estilo do Krita: pintura, camadas, seleções, filtros e dockers que você arruma como quiser.

- **Dockers:** Ferramentas, Camadas, Pincéis, Editor de pincel, Opções da ferramenta, Cor, Paleta, Visão geral e
  Histórico. Arraste uma aba para outro lugar, junte em abas, solte numa janela, feche; Configurações → Dockers
  mostra de novo, e o layout fica salvo. Vários documentos abertos em abas.
- **Pincéis:** motor de carimbos com tamanho, dureza, opacidade (teto do traço), fluxo (soma), espaçamento, ângulo,
  achatamento, grão, dispersão e estabilizador. Vêm 7 prontos (Básico, Aerógrafo, Lápis, Tinta, Marcador, Giz,
  Aquarela) e dá para salvar os seus. `[` e `]` mudam o tamanho; Ctrl+clique pega a cor.
- **Ferramentas:** pincel (B), borracha (E), balde de tinta (F), degradê (G), conta-gotas (P), seleção retangular (J),
  elíptica (K), laço (L) e varinha mágica (W), mover (T), transformar (Ctrl+T), cortar (C), linha (I),
  retângulo (U), elipse (O), borrão (S) e texto (Q). X troca frente e fundo, D volta ao preto e branco.
- **Camadas:** pixels, 13 modos de mistura, opacidade, travar e travar alfa, mostrar/esconder, duplicar, unir,
  reordenar, importar imagem como camada. Copiar, recortar e colar entre documentos.
- **Seleções:** somar (Shift), subtrair (Alt), inverter, suavizar a borda, expandir/contrair; limitam pincel,
  preenchimento, filtros e transformação.
- **Filtros:** brilho/contraste, matiz/saturação, níveis, limiar, posterizar, inverter, dessaturar, desfoque,
  nitidez, pixelar, ruído e detecção de bordas, com prévia.
- **Imagem:** tamanho da tela, escalar, cortar, girar e espelhar, achatar.
- **Arquivos:** abre PNG, JPG, WebP e BMP; salva `.pawpaint` (um zip com uma camada por PNG); exporta PNG, JPG, WebP
  ou BMP, e *JPG com até 2 MB* para thumbnails do YouTube.
- **Desfazer:** guarda o documento inteiro a cada passo sem gastar memória (os blocos de pixels são compartilhados),
  até 60 passos ou 1,5 GB de pixels retidos, o que vier primeiro.

- **Mais:** pastas de camadas, máscaras de camada (preto esconde, branco revela), pintura com espelho, borrão, curvas, colar prints da
  área de transferência do sistema, recuperação automática se o app fechar de repente.
- **Fotos grandes sem travar:** filtros, preenchimento e exportação usam todos os núcleos (um desfoque numa foto de
  12 megapixels leva ~0,5 s); a prévia de um filtro lento só é refeita ao soltar o slider; o autosave e o salvamento
  rodam em segundo plano com uma cópia barata do documento; o carimbo do pincel ficou ~2x mais rápido, o que pesa
  nos pincéis grandes.
- **Caixa de ferramentas como a do Photoshop:** diminua o painel e ela vira uma coluna só (ou fixe
  uma coluna no botão de setinhas no topo). Arrastar a tela é Espaço + arrastar (o botão do meio
  abre a roda de atalhos).
- **Janelas pequenas:** a barra do pincel recolhe o excesso num menu "⋯", os dockers rolam e os diálogos mantêm a
  largura. Nos diálogos, Esc cancela e Enter confirma.

- **Dinâmica do pincel:** o tamanho e o fluxo mudam com a **pressão** da caneta (quando o sistema entrega), a
  **velocidade**, a distância (o traço some aos poucos) ou sorteio, e o ângulo pode seguir o traço. Vêm pincéis
  prontos (Caneta tinteiro, Pincel de pressão, Traço que some, Caligrafia, Spray). Shift+clique traça uma reta
  desde onde o último traço terminou. Alt+arrastar na horizontal muda o tamanho do pincel, com o círculo e o valor na tela.
- **Régua:** *Ver → Colocar régua* e arraste na imagem; os traços que começam perto da reta ficam presos a ela
  (linhas retas e perspectiva sem tremer). *Tirar a régua* remove.
- **Camadas de ajuste:** brilho/contraste, matiz/saturação, níveis, curvas, limiar, posterizar, preto e branco e
  inverter agem em tudo que está embaixo sem tocar nos pixels; a opacidade e a máscara da camada limitam onde.
  Clique duas vezes para editar. Camada → Nova camada de ajuste.
- **Carimbo de clone (H):** Alt+clique escolhe de onde copiar e a cruz mostra o que está sendo copiado.
- **Visão:** girar (Ctrl+] e Ctrl+[, ou Shift+Espaço e arrastar), zerar (5) e espelhar (M) sem mexer na imagem;
  modo de repetição/mosaico (Shift+W) para texturas que emendam; simetria radial (Opções da ferramenta).
- **Painel Referência:** uma imagem ao lado da tela (fora do documento) em que dá para clicar e pegar cores.
- **Dockers largos de propósito:** a coluna da direita e a caixa de ferramentas nascem com largura fixa em pixels,
  e títulos de aba que não cabem viram `Ferram…` (sem barra de rolagem).

Ainda não tem: camadas vetoriais, histograma e assistentes de desenho (régua, perspectiva).

---

# PawCut

Editor de vídeo simples para Linux, com a cara da página **Edit do DaVinci Resolve**.
Serve para cortar, empilhar camadas, pôr overlays e títulos, mexer no áudio e exportar MP4,
com keyframes e ajuste de cor no estilo do Ctrl+U do Photoshop.

Feito em Rust: interface em egui, mídia com ffmpeg. Um único compositor gera o preview e a
exportação, então o MP4 sai igual ao que aparece no viewer.

## Rodar

```bash
scripts/install.sh            # compila e instala (menu de aplicativos + ~/.local/bin/pawcut)
pawcut                        # ou: pawcut projeto.pawcut / pawcut video.mp4 musica.mp3
```

Para rodar sem instalar: `cargo run --release -p pawcut-app`.

Requisitos (Ubuntu): Rust (rustup), `ffmpeg` com as bibliotecas de desenvolvimento
(`libavcodec-dev`, `libavformat-dev`, `libavfilter-dev`, `libswscale-dev`), PulseAudio/PipeWire
e libclang. Se faltar o `libclang-dev`, o `.cargo/config.toml` aponta o bindgen para os headers do GCC.

## O que dá para fazer

- **Media Pool:** importar vídeo, áudio e imagem pelo botão, com Ctrl+I ou arrastando arquivos para a janela.
  Arquivo que sumiu aparece como *Media Offline*; clique com o botão direito para reconectar (Relink).
- **Effects → Toolbox:** arraste *Texto* ou *Cor sólida* para uma faixa de vídeo.
- **Source Viewer:** clique numa mídia, marque In/Out (I/O) e use Insert (F9), Overwrite (F10) ou Append (Shift+F12).
- **Timeline:** várias faixas de vídeo (a de cima cobre a de baixo) e de áudio. Dá para arrastar
  clipes no tempo e entre faixas (soltar em "+ vídeo"/"+ áudio" cria a faixa), aparar pelas bordas,
  usar a lâmina (B) ou cortar no cursor (Ctrl+B), apagar com ou sem ripple, e usar snapping (N) e seleção ligada.
  Cada faixa tem botões para travar (L), ligar/desligar o vídeo (V) e mudo/solo no áudio (M/S).
  Ctrl+roda dá zoom e a roda rola a timeline.
- **Timeline Viewer:** arraste a imagem para mover o clipe selecionado.
- **Inspector:** posição, zoom e opacidade, velocidade (10%–1000%, o áudio mantém o tom), fade in/out,
  volume em dB, texto (conteúdo, tamanho, fonte, cor) e cor do gerador.
- **Áudio:** cada vídeo entra com o áudio ligado. "Separar áudio e vídeo" (Ctrl+Alt+L) desliga os dois.
  A waveform aparece na timeline.
- **Deliver:** exporta MP4 (H.264 + AAC) com barra de progresso e opção de cancelar.
- **Projeto:** salva em `.pawcut` (JSON). Há autosave a cada 5 minutos e recuperação se o app fechar sem salvar.
- **Layout:** os painéis são abas que podem ser arrastadas, divididas e fechadas.
  O menu *Workspace* reabre painéis e tem *Reset UI Layout*.
- **Notas:** o painel *Notas* (menu Workspace) guarda anotações livres do projeto, salvas no arquivo e desfazíveis.
- **Atalhos editáveis:** *Editar → Preferences*. Os padrões são os do Resolve.

## Atalhos padrão

| Ação | Tecla | Ação | Tecla |
|---|---|---|---|
| Tocar/pausar | Espaço | J / K / L | trás / parar / frente (repita para acelerar) |
| Frame anterior/seguinte | ← / → | Corte anterior/seguinte | ↑ / ↓ |
| Início / fim | Home / End | In / Out | I / O |
| Seleção / Lâmina | A / B | Snapping | N |
| Cortar no cursor | Ctrl+B | Seleção ligada | Ctrl+Shift+L |
| Apagar | Backspace / Delete | Apagar com ripple | Shift+Backspace |
| Insert / Overwrite / Append | F9 / F10 / Shift+F12 | Ligar/separar A/V | Ctrl+Alt+L |
| Desfazer / Refazer | Ctrl+Z / Ctrl+Shift+Z | Salvar / Salvar como | Ctrl+S / Ctrl+Shift+S |
| Novo / Abrir | Ctrl+N / Ctrl+O | Importar | Ctrl+I |
| Exportar (Deliver) | Ctrl+Shift+E | Zoom + / − / caber | Ctrl+= / Ctrl+- / Shift+Z |
| Cortar até o cursor (ripple) | Shift+[ / Shift+] | Copiar / Colar | Ctrl+C / Ctrl+V |
| Marcador / anterior / próximo | M / Shift+↑ / Shift+↓ | Cross Dissolve | Ctrl+T |

## Recursos de edição

- **Prévia ao passar o mouse:** passe o mouse numa mídia do Media Pool e o viewer mostra o vídeo naquela posição, como o hover scrub do Resolve. Clique para usar como Source (In/Out, Insert, Overwrite).
- **Cortes rápidos:** Shift+[ e Shift+] cortam do início até o cursor (ou do cursor até o fim) e fecham o buraco.
- **Copiar e colar** clipes com Ctrl+C/Ctrl+V. O áudio e o vídeo ligados vão juntos.
- **Marcadores:** M cria, Shift+↑/↓ pula entre eles, dois cliques na bandeirinha editam nota e cor.
- **Fades:** arraste os quadradinhos nos cantos de cima do clipe.
- **Cross Dissolve:** Ctrl+T no corte mais perto do cursor, ou arraste da Toolbox até um corte. O áudio ganha um crossfade.
- **Keyframes:** o botão ◆ no Inspector anima posição, zoom, opacidade, volume e cor. Os losangos aparecem no clipe selecionado.
- **Cor no estilo Ctrl+U do Photoshop:** matiz, saturação e luminosidade por clipe.
- **Recortar e espelhar** a imagem no Inspector.
- **Títulos:** negrito, alinhamento, contorno, sombra e caixa de fundo. Há presets de Título, Lower third e Legenda.
- **Áudio:**
  - medidores L/R no viewer;
  - "Normalizar" (pico em −1 dB);
  - "Remover silêncios", que corta as pausas da fala e fecha os buracos, com o vídeo junto.
- **Exportação:**
  - presets YouTube 1080p e 4K (H.265), 720p rápido, e só áudio em MP3 ou WAV;
  - exportar só o trecho In/Out;
  - salvar o frame atual em PNG.
- **Mídia otimizada (proxy):** clique direito na mídia, para vídeos 4K de celular rodarem lisos no preview. A exportação usa sempre o original.
- **Áudio por clipe:** na aba Áudio do Inspector, *Efeitos de áudio* liga filtro de graves, noise gate, notch,
  de-esser, equalizador, compressor e reverb no clipe (o botão *Limpar voz* liga os quatro principais). Rodam no
  áudio inteiro do clipe, tanto na prévia quanto na exportação.
- **Música abaixa na voz** (Inspector → Música abaixa na voz): cria keyframes de volume na música quando os outros
  clipes de áudio falam, e dá para ajustar à mão depois.
- **Alinhar pelo áudio** (Ctrl+Shift+J): selecione a câmera primeiro e o áudio do microfone depois; ele acha o mesmo
  som nos dois e move o segundo até casar (com o vídeo ligado junto).
- **Legendas automáticas** (Ctrl+Shift+T, precisa do `whisper` instalado): transcreve o clipe selecionado e cria a
  faixa de legendas na timeline.
- **Rampa de velocidade** (Inspector → Rampa): acelera ou freia aos poucos; o clipe vira pedaços e o resto da faixa
  acompanha. **Panorâmica** (Inspector → Panorâmica): preenche o quadro e desliza de um lado ao outro (vertical 9:16).
- **Presets de aparência:** salve a cor, o volume, o chroma key e o fundo desfocado de um clipe com um nome e aplique
  em outros (ficam em `presets.json`).
- **Slideshow** (menu Timeline): põe as imagens do Media Pool em ordem com dissolve. Em Deliver, *Um arquivo por
  marcador* exporta cada pedaço entre marcadores num arquivo numerado (capítulos, Shorts).
- **Configurações do projeto:** mude para vertical 9:16, quadrado ou 4K. Os projetos recentes ficam no menu Arquivo e na tela vazia do Media Pool.

## Onde ficam os arquivos

- Atalhos e layout: `~/.config/pawcut/`
- Autosave: `~/.local/share/pawcut/autosave/`
- Cache de áudio decodificado: `~/.cache/pawcut/` (pode apagar à vontade)

## Desenvolvimento

```bash
cargo test --workspace        # 140+ testes, incluindo cliques e arrastos simulados na interface
cargo clippy --workspace --all-targets -- -D warnings
scripts/ui-fuzz.sh pawcut 300 1 -- video.mp4   # cliques e atalhos aleatórios num display virtual; avisa se cair
scripts/ui-fuzz.sh pawpaint 400 1 -- foto.png  # o mesmo para qualquer app (pawvoice, pawwrite, pawpaint)

# Medidas de desempenho (sempre com --release e a máquina quieta)
cargo run --release -p pawpaint-core --example bench_ops    # filtros, preencher, achatar, pincel
cargo run --release -p pawpaint-core --example bench_save   # salvar e autosave de foto grande
cargo run --release -p pawwrite-core --example bench_text   # buscar e substituir em texto grande
cargo run --release -p pawvoice-core --example bench_mix    # mixagem ao vivo
cargo run --release -p paw-audio --example bench_dsp        # efeitos de áudio
```

Os crates são:
- `pawcut-core`: modelo, edições, desfazer/refazer e arquivo `.pawcut`.
- `paw-media`: lê arquivos com ffmpeg.
- `pawcut-render`: compositor e mixer.
- `pawcut-export`: MP4.
- `pawcut-app`: a interface.

Exemplos de linha de comando: `cargo run -p pawcut-export --example render_project -- projeto.pawcut saida.mp4`.
