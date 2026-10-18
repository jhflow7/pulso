# Pulso — Download Manager

Um gerenciador de downloads desktop para Linux, feito com **Tauri 2 + React + TypeScript + Rust**, usando o `aria2c` como motor de transferência.

> Protótipo funcional inicial (v0.1.0). O diretório padrão é `~/Downloads`. Os downloads são processados pelo aria2 em segundo plano enquanto o Pulso está aberto.

## O que já funciona

- Adicionar links HTTP/HTTPS, FTP, SFTP e magnet.
- Ver transferências em tempo real, velocidade, tamanho, progresso, conexões e status.
- Pausar, retomar e remover downloads.
- Pausar/retomar a fila e limpar itens finalizados do histórico.
- Retomar downloads e salvar a sessão do aria2.
- Abrir a pasta de downloads pelo gerenciador de arquivos.
- RPC restrito a `127.0.0.1`, com porta dinâmica e token gerado a cada inicialização.
- Perfil rápido inicial: até 5 downloads simultâneos, split 16, máximo de 16 conexões por servidor e `min-split-size=1M`.

## 1. Instalar as dependências no CachyOS

No terminal:

```bash
sudo pacman -Syu
sudo pacman -S --needed base-devel webkit2gtk-4.1 curl wget file openssl appmenu-gtk-module libappindicator-gtk3 librsvg xdotool nodejs npm rustup aria2
```

Configure a toolchain Rust caso ainda não tenha feito:

```bash
rustup default stable
```

Confira as ferramentas:

```bash
node --version
npm --version
rustc --version
cargo --version
aria2c --version
```

## 2. Abrir no Visual Studio Code

Extraia o projeto, abra a pasta `pulso-fdm` no VS Code e abra o terminal integrado pelo menu **Terminal → New Terminal**. Se extraiu para `~/Projetos`, por exemplo:

```bash
cd ~/Projetos/pulso-fdm
npm install
```

## 3. Executar em modo desenvolvimento

```bash
npm run tauri dev
```

Na primeira compilação, o Cargo baixa e compila as dependências Rust. O Pulso abre como uma aplicação desktop. Não rode `npm run dev` sozinho para testar a integração completa: no navegador, os comandos nativos do Tauri não existem.

## 4. Gerar pacote instalável

```bash
npm run tauri build
```

Os artefatos normalmente aparecem em:

- `src-tauri/target/release/bundle/appimage/`
- `src-tauri/target/release/bundle/deb/`
- `src-tauri/target/release/bundle/rpm/`

O build de distribuição compila o frontend e o binário Rust; o `aria2c` continua sendo uma dependência do sistema e precisa estar instalado no computador onde o Pulso for executado.

## 5. Ajustar o desempenho

As opções principais ficam em `src-tauri/src/lib.rs`, dentro da inicialização do `Command::new("aria2c")`:

```rust
.arg("--max-concurrent-downloads=5")
.arg("--max-connection-per-server=16")
.arg("--split=16")
.arg("--min-split-size=1M")
```

E a mesma configuração por download está em `add_download`, no objeto `options`.

- `max-concurrent-downloads=5`: até cinco downloads em paralelo.
- `max-connection-per-server=16` e `split=16`: tenta baixar arquivos divididos em paralelo, quando o servidor suporta requisições por intervalo (HTTP Range).
- `min-split-size=1M`: evita dividir em partes pequenas demais.
- `file-allocation=none`: começa sem pré-alocar todo o tamanho do arquivo.

**Mais conexões não significam automaticamente mais velocidade.** Servidores podem limitar ou bloquear conexões paralelas; o gargalo também pode ser sua rede, o Wi-Fi, o disco ou o servidor de origem. Por isso, teste a configuração com arquivos grandes em servidores diferentes antes de subir os limites.

## Diagnóstico

- Se aparecer que o motor não iniciou: confira `aria2c --version` e instale com `sudo pacman -S aria2`.
- O log do aria2 é salvo na pasta de dados do app do Tauri, normalmente em `~/.local/share/dev.pulso.fdm/aria2.log`.
- Para erro de build relacionado ao WebKit, confirme que `webkit2gtk-4.1` está instalado e que o sistema está atualizado.

## Próximos passos recomendados

Para evoluir esse protótipo: seletor de pasta nativo, renomear arquivo antes de começar, limite de banda, fila editável, notificações do KDE, integração de captura de links do navegador e testes com downloads interrompidos. O projeto ainda não inclui integração com extensões de navegador nem suporte de UI dedicado para todos os recursos BitTorrent do aria2.
