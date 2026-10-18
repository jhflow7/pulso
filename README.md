# Pulso 0.2.0 — Download Studio

Pulso é um gerenciador de downloads desktop para Linux feito com **Tauri 2, React, TypeScript e Rust**. O aria2 continua sendo o motor de transferência; a aplicação fornece uma interface local, fila, histórico, estimativas, ajustes automáticos e integração com o desktop.

> **Estado:** versão prévia para teste. O frontend e a configuração foram revisados estaticamente; antes de publicar um binário, execute os testes e valide uma compilação no teu CachyOS.

## O que há nesta versão

- **Nome e identidade consistentes:** o aplicativo chama-se Pulso; pacote Rust e biblioteca foram renomeados para `pulso` e `pulso_lib`.
- **Downloads diretos:** HTTP/HTTPS, FTP e SFTP por link, links em lote e magnet links.
- **Arquivo `.torrent`:** botão para escolher um arquivo local, enviá-lo ao aria2 com `aria2.addTorrent`, acompanhar sua fila e tentar novamente uma falha se o arquivo original continuar no mesmo caminho.
- **Pasta de destino:** diretório padrão persistente e destino opcional por transferência/lote.
- **Pulso Auto:** ajuste inicial do número de conexões/divisões com base no tamanho e nas pistas do servidor quando disponíveis; fallback para heurísticas quando não. Não garante acelerar todo servidor.
- **Perfis manuais:** Economia, Equilibrado, Turbo e personalizado; limites de download/upload e notificações de conclusão.
- **Fila e histórico:** pausar, retomar, repetir falhas, limpar histórico e recuperar dados locais.
- **ETA com suavização:** tempo restante estimado a partir do tamanho pendente e da velocidade recente.
- **Segurança:** RPC do aria2 restrito a `127.0.0.1`, segredo gerado para cada execução, pasta de dados com permissões privadas no Linux e verificação de certificados HTTPS. O Pulso não desliga TLS para tentar ganhar velocidade.
- **Migração de dados:** na primeira execução com o novo identificador da aplicação, tenta copiar arquivos de configuração/sessão do perfil antigo sem sobrescrever um perfil novo existente.
- **Limpeza de estado:** remove metadados de tuning obsoletos do registro auxiliar depois que um download termina; o histórico retém os dados apresentados na interface.

## O aria2 é a escolha certa?

Para um gerenciador que combina downloads HTTP/HTTPS, FTP/SFTP, magnet e `.torrent`, sim: é um motor leve e com RPC bem estabelecido. Não é necessariamente a melhor opção para um cliente de torrents altamente especializado, em que controles avançados de peers, trackers e seleção por arquivo podem justificar outro motor. O Pulso separa a UI do motor por meio do backend Rust para deixar essa escolha evolutiva.

## Dependências no CachyOS / Arch Linux

```bash
sudo pacman -Syu
sudo pacman -S --needed \
  base-devel nodejs npm rustup aria2 \
  webkit2gtk-4.1 curl wget file openssl librsvg xdg-utils kdialog
rustup default stable
```

`kdialog` é usado para selecionar pastas e arquivos `.torrent` no KDE. Se preferires, podes usar `zenity` como alternativa. O motor `aria2c` precisa estar instalado e acessível no `PATH`; o Pulso não inclui um binário do aria2 no AppImage por padrão.

Para aplicar esta versão no repositório Git já criado para a 0.1.0, siga [`MIGRACAO-0.1.0.md`](MIGRACAO-0.1.0.md) e trabalhe primeiro numa branch de teste.

## Rodar em desenvolvimento

```bash
npm install
npm run tauri dev
```

`npm run dev` sozinho executa apenas a interface web, sem os comandos Tauri/Rust.

## Verificações antes de publicar

O workflow em `.github/workflows/ci.yml` executa a compilação/type-check do frontend e os testes Rust a cada push ou pull request na branch `main`. Ele ajuda a encontrar erros de integração, mas não substitui o teste manual da janela e de transferências reais.

```bash
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
npm run tauri build
```

Teste no desktop: abrir a aplicação, adicionar um link HTTP(S), pausar/retomar, adicionar um arquivo `.torrent` que você tenha direito de compartilhar, mudar a pasta, fechar/abrir de novo e tentar novamente uma falha.

## Compilar e localizar os pacotes

```bash
npm run tauri build
find src-tauri/target/release/bundle -maxdepth 3 -type f
```

Os alvos de distribuição estão configurados para AppImage, DEB e RPM. O AppImage tem que ser testado num sistema de destino limpo: dependendo da distribuição e do pacote, o usuário também pode precisar instalar `aria2`.

## Dados locais e privacidade

As preferências, a sessão, o histórico, as origens dos links e o log ficam no diretório de dados da aplicação. URLs podem conter tokens temporários; não compartilhe `sources.json` ou o log sem revisá-los. A RPC do aria2 é local e protegida por token.

## Limitações conhecidas

- Ainda não há extensão de navegador, verificação de checksum pela UI, agendamento, seleção de arquivos dentro de torrents ou configurações avançadas de semeadura. Nesta versão, um URL HTTP que aponta para um `.torrent` remoto baixa o próprio arquivo; para abrir um torrent, selecione o `.torrent` local pelo botão de arquivo ou use um magnet link.
- O modo automático escolhe heurísticas iniciais; ele não mede um benchmark por host, não aprende entre sessões e não pode contornar limites do servidor.
- O ETA varia com a rede, o servidor, a concorrência e a disponibilidade de peers.
- Os testes unitários não substituem testes reais de transferência.

## Licença

A licença precisa ser escolhida pelo mantenedor antes de a distribuição ser apresentada como open source reutilizável.
