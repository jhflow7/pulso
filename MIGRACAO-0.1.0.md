# Migrar o Pulso 0.1.0 para o código 0.2.0

Este pacote é uma árvore de código completa para testar em separado. Ele não altera o repositório do GitHub por conta própria.

## 1. Testar sem arriscar a versão atual

Extraia `Pulso-v0.2.0.zip` para a sua pasta pessoal e execute na pasta extraída:

```bash
npm install
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
npm run tauri dev
```

Teste um link HTTP/HTTPS, pausa/retomada, seleção da pasta, um magnet e um arquivo `.torrent` local que você tenha direito de compartilhar. Feche e reabra o Pulso para verificar a retomada e o histórico.

## 2. Aplicar no repositório existente sem perder o histórico Git

Só depois de os testes passarem, copie o código para a cópia de trabalho Git existente. Isso preserva a pasta `.git` e o histórico da 0.1.0.

```bash
cd ~/pulso-v0.1.0
git status
git switch -c release/v0.2.0
```

Certifique-se de que não há alterações suas sem commit. Em seguida, se tiver `rsync` instalado:

```bash
rsync -av \
  --exclude='.git/' \
  --exclude='node_modules/' \
  --exclude='dist/' \
  --exclude='src-tauri/target/' \
  ~/Pulso-v0.2.0/ ~/pulso-v0.1.0/
```

Se a pasta extraída tiver outro caminho, ajuste `~/Pulso-v0.2.0/` para o caminho real. Este comando substitui os arquivos de código/configuração, mas não copia a pasta Git nem os grandes artefatos de build.

Depois, dentro de `~/pulso-v0.1.0`:

```bash
npm install
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
npm run tauri dev
```

Se tudo passar:

```bash
npm run tauri build
git status --short
git add -A
git commit -m "feat: Pulso 0.2.0 with torrent support"
git push -u origin release/v0.2.0
```

Revise a branch e o resultado do workflow **Pulso CI** no GitHub antes de criar a tag `v0.2.0` e uma Release pública. Não publique um binário que não tenha sido testado localmente.

## Observação sobre dados

O identificador Tauri muda para `dev.pulso.desktop`. Na primeira inicialização, o backend tenta copiar os arquivos conhecidos do perfil anterior sem sobrescrever um `settings.json` novo. Não apague a pasta de dados antiga até confirmar que preferências e downloads pendentes foram restaurados.
