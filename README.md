# Gerenciador visual de SSH em Rust

Aplicação didática feita com [`egui`](https://github.com/emilk/egui) e `eframe`
para visualizar e editar blocos `Host` de um arquivo de configuração SSH.

Por padrão, o programa abre diretamente `~/.ssh/config`. O arquivo somente é
gravado quando você aplica as alterações e clica em **Salvar arquivo**.

## Executar

Na pasta do projeto:

```sh
cargo run
```

Também é possível informar outro arquivo explicitamente:

```sh
cargo run -- caminho/do/arquivo
```

Para testar sem usar seu arquivo real:

```sh
cargo run -- config_sample
```

## Fluxo da interface

1. Se necessário, clique em **Abrir arquivo…** para escolher outro arquivo pelo
   seletor nativo do sistema.
2. Escolha um Host na lista da esquerda.
3. Altere os campos no formulário.
4. Clique em **Aplicar alterações** para atualizar o documento em memória.
5. Confira **Prévia do arquivo**.
6. Clique em **Salvar** para gravar.

O seletor de arquivo fica desabilitado enquanto houver alterações pendentes,
evitando que uma troca de arquivo descarte edições ainda não salvas.

Antes de salvar, o programa cria um arquivo com a extensão `.bak`. Ao editar o
arquivo padrão, o backup será `~/.ssh/config.bak`.

## Proteções implementadas

- O arquivo `~/.ssh/config` só é gravado após confirmação no botão Salvar.
- Campos não aceitam quebras de linha.
- A porta precisa estar entre 1 e 65535.
- Aliases duplicados bloqueiam o salvamento.
- Comentários e diretivas não exibidas no formulário são preservados.
- A ordem dos blocos Host é preservada.
- Arquivos com uma seção `Match` podem ser vistos, mas não salvos.
- Toda gravação cria primeiro uma cópia `.bak`.

## Testes

```sh
cargo test
```

Os testes verificam o parser, a preservação de opções desconhecidas, a edição,
a validação de portas e a detecção de aliases duplicados.

## Criar um instalador para macOS

O script de empacotamento requer macOS, Rust e as Command Line Tools do Xcode.
Execute:

```sh
./scripts/build-macos-pkg.sh
```

O resultado será criado em `dist/manager-config-file-VERSAO-ARQUITETURA.pkg`.
Abra esse arquivo no Finder para instalar **Gerenciador SSH.app** em
`/Applications`.

O pacote sem certificados recebe uma assinatura ad hoc e é indicado somente
para instalação local. Para criar um pacote assinado para distribuição, informe
os nomes dos certificados instalados no Keychain:

```sh
DEVELOPER_ID_APPLICATION="Developer ID Application: Seu Nome (TEAMID)" \
DEVELOPER_ID_INSTALLER="Developer ID Installer: Seu Nome (TEAMID)" \
./scripts/build-macos-pkg.sh
```

Para distribuição fora da App Store, o pacote assinado também deve passar pela
notarização da Apple.

Quando aberto pelo Finder, o aplicativo carrega diretamente `~/.ssh/config`.
Um caminho passado pela linha de comando continua tendo prioridade.

## Conceitos de Rust usados

O arquivo `main.rs` contém comentários em português junto ao código para
explicar:

- structs e implementações com `impl`;
- propriedade, empréstimos e referências mutáveis;
- `Option`, `Result` e tratamento de erros;
- vetores, strings e `HashMap`;
- leitura, cópia e escrita de arquivos;
- implementação do trait `eframe::App`;
- construção de widgets e tratamento de cliques no egui;
- testes unitários.
