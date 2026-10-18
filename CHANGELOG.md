# Changelog — Pulso 0.2.0

## 0.2.0 — primeira revisão focada em confiabilidade

### Identidade e manutenção
- Adicionado guia de migração para atualizar a cópia Git da 0.1.0 sem perder o histórico.
- Renomeada a biblioteca Rust para `pulso_lib`, alinhada com o nome comercial Pulso.
- Atualizados os metadados do npm, Cargo, Tauri, interface e documentação para `0.2.0`.
- Alterado o identificador para `dev.pulso.desktop`; na primeira inicialização, o backend tenta migrar preferências, histórico, origens, sessão e log do perfil anterior, sem sobrescrever um perfil novo.
- Corrigida a prioridade do nome BitTorrent: a interface usa o nome do torrent em vez do nome do primeiro arquivo interno.
- Metadados auxiliares de tuning são eliminados depois da persistência do histórico terminal, evitando acúmulo desnecessário.

### BitTorrent
- Adicionado seletor gráfico de arquivos `.torrent` com `kdialog` e fallback `zenity`.
- Adicionada integração `aria2.addTorrent` com metainfo codificada em Base64.
- Arquivos vazios, caminhos inexistentes, extensões erradas e metainfo maior que 32 MiB são rejeitados antes de chamar o motor.
- A origem local é registrada para permitir nova tentativa enquanto o `.torrent` original continuar disponível.
- Magnet links existentes continuam funcionando pela API normal `aria2.addUri`.

### Testes e qualidade
- Adicionados testes de round-trip para registrar e recuperar origens de torrents locais.
- Adicionado workflow de CI do GitHub para compilar/type-check o frontend e executar os testes Rust em Ubuntu.
- Preservada a cobertura de validação de links, nomes de arquivo, migração de preferências e heurísticas do Pulso Auto.
- Mantidas as verificações TLS e a RPC no loopback; não foi introduzida nenhuma otimização que desative certificados.
- Restringidas no Linux as permissões do diretório de dados para o usuário local (`0700`).

### Limitações conhecidas
- Não há ainda seleção visual dos arquivos internos de um torrent, extensão de navegador, agendamento ou benchmark adaptativo por servidor.
- O resultado final deve ser compilado e testado no CachyOS antes de se publicar a Release.
