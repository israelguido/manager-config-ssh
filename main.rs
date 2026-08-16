// `eframe` cria a janela nativa e reexporta a biblioteca visual `egui`.
use eframe::egui;
// Tipos da biblioteca padrão usados para arquivos e caminhos.
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

// Arquivo didático usado somente se a variável HOME não estiver disponível.
const FALLBACK_CONFIG: &str = "config_sample";

// Ponto inicial do programa.
fn main() -> eframe::Result {
    // Permite abrir outro arquivo com: `cargo run -- caminho/do/arquivo`.
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);

    // Configura o tamanho inicial da janela.
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1_050.0, 700.0]),
        // Encerra o processo junto com a janela. No macOS com Touch Bar, deixar
        // o event loop retornar pode disparar uma exceção tardia do AppKit.
        run_and_return: false,
        ..Default::default()
    };

    // `run_native` mantém o programa executando e redesenha a GUI quando necessário.
    eframe::run_native(
        "Gerenciador SSH em Rust",
        native_options,
        Box::new(move |creation_context| {
            configure_style(&creation_context.egui_ctx);
            Ok(Box::new(ManagerApp::load(path)))
        }),
    )
}

// Sem argumento explícito, abre diretamente o arquivo SSH do usuário atual.
fn default_config_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".ssh").join("config"))
        .unwrap_or_else(|| PathBuf::from(FALLBACK_CONFIG))
}

// Representa todo o arquivo SSH em memória.
#[derive(Default)]
struct ConfigDocument {
    // Linhas globais encontradas antes do primeiro `Host`.
    prefix: Vec<String>,
    // Blocos Host na mesma ordem em que aparecem no arquivo.
    hosts: Vec<HostEntry>,
    // O editor não salva arquivos com Match, para evitar mudar sua semântica.
    contains_match: bool,
}

// Um bloco Host guarda seu cabeçalho e todas as linhas internas originais.
#[derive(Clone)]
struct HostEntry {
    names: String,
    body: Vec<String>,
}

impl HostEntry {
    // Cria um bloco inicial quando o botão "Novo host" é pressionado.
    fn new() -> Self {
        Self {
            names: "novo-host".to_owned(),
            body: vec![
                "    HostName servidor.exemplo.com".to_owned(),
                "    User usuario".to_owned(),
                "    Port 22".to_owned(),
                "    IdentityFile ~/.ssh/id_ed25519".to_owned(),
            ],
        }
    }

    // Procura o primeiro valor de uma diretiva, como `User` ou `Port`.
    fn option(&self, searched_key: &str) -> String {
        self.body
            .iter()
            .filter_map(|line| split_directive(line))
            .find(|(key, _)| key.eq_ignore_ascii_case(searched_key))
            .map(|(_, value)| value.to_owned())
            .unwrap_or_default()
    }

    // Atualiza uma diretiva sem apagar comentários ou opções desconhecidas.
    fn set_option(&mut self, searched_key: &str, new_value: &str) {
        let matching_indexes: Vec<usize> = self
            .body
            .iter()
            .enumerate()
            .filter_map(|(index, line)| {
                let (key, _) = split_directive(line)?;
                key.eq_ignore_ascii_case(searched_key).then_some(index)
            })
            .collect();

        // Um campo vazio remove essa opção do bloco.
        if new_value.trim().is_empty() {
            self.body = self
                .body
                .drain(..)
                .enumerate()
                .filter_map(|(index, line)| (!matching_indexes.contains(&index)).then_some(line))
                .collect();
            return;
        }

        let formatted_line = format!("    {searched_key} {}", new_value.trim());

        if let Some(first_index) = matching_indexes.first().copied() {
            // Substitui a primeira ocorrência.
            self.body[first_index] = formatted_line;
            // Remove repetições da mesma opção, começando pela última posição.
            for index in matching_indexes.into_iter().skip(1).rev() {
                self.body.remove(index);
            }
        } else {
            // Se a opção ainda não existe, adiciona-a no fim do bloco.
            self.body.push(formatted_line);
        }
    }
}

// Campos temporários apresentados no formulário da direita.
#[derive(Clone, Default, PartialEq)]
struct HostEditor {
    names: String,
    host_name: String,
    user: String,
    port: String,
    identity_file: String,
}

impl HostEditor {
    // Copia os dados de um Host para os campos visuais.
    fn from_host(host: &HostEntry) -> Self {
        Self {
            names: host.names.clone(),
            host_name: host.option("HostName"),
            user: host.option("User"),
            port: host.option("Port"),
            identity_file: host.option("IdentityFile"),
        }
    }

    // Verifica dados perigosos ou inválidos antes de aplicá-los.
    fn validate(&self) -> Result<(), String> {
        if self.names.trim().is_empty() {
            return Err("O campo Host não pode ficar vazio.".to_owned());
        }

        // Uma quebra de linha permitiria injetar outra diretiva no arquivo.
        for (label, value) in [
            ("Host", &self.names),
            ("HostName", &self.host_name),
            ("User", &self.user),
            ("Port", &self.port),
            ("IdentityFile", &self.identity_file),
        ] {
            if value.contains(['\n', '\r']) {
                return Err(format!("{label} não pode conter quebra de linha."));
            }
        }

        if !self.port.trim().is_empty() && self.port.trim().parse::<u16>().is_err() {
            return Err("Port deve ser um número entre 1 e 65535.".to_owned());
        }

        if self.port.trim() == "0" {
            return Err("Port deve ser maior que zero.".to_owned());
        }

        Ok(())
    }

    // Transfere os campos validados de volta para o documento.
    fn apply_to(&self, host: &mut HostEntry) {
        host.names = self.names.trim().to_owned();
        host.set_option("HostName", &self.host_name);
        host.set_option("User", &self.user);
        host.set_option("Port", &self.port);
        host.set_option("IdentityFile", &self.identity_file);
    }
}

// Cor e texto de uma mensagem apresentada no rodapé.
enum StatusKind {
    Information,
    Success,
    Warning,
    Error,
}

struct StatusMessage {
    kind: StatusKind,
    text: String,
}

impl StatusMessage {
    fn information(text: impl Into<String>) -> Self {
        Self {
            kind: StatusKind::Information,
            text: text.into(),
        }
    }

    fn success(text: impl Into<String>) -> Self {
        Self {
            kind: StatusKind::Success,
            text: text.into(),
        }
    }

    fn warning(text: impl Into<String>) -> Self {
        Self {
            kind: StatusKind::Warning,
            text: text.into(),
        }
    }

    fn error(text: impl Into<String>) -> Self {
        Self {
            kind: StatusKind::Error,
            text: text.into(),
        }
    }
}

// Estado completo da aplicação gráfica.
struct ManagerApp {
    path: PathBuf,
    document: ConfigDocument,
    selected: Option<usize>,
    editor: HostEditor,
    // Texto usado para filtrar rapidamente a lista lateral.
    search: String,
    dirty: bool,
    confirm_delete: bool,
    status: StatusMessage,
}

impl ManagerApp {
    // Constrói a aplicação e tenta abrir o arquivo imediatamente.
    fn load(path: PathBuf) -> Self {
        match read_document(&path) {
            Ok(document) => {
                let selected = (!document.hosts.is_empty()).then_some(0);
                let editor = selected
                    .map(|index| HostEditor::from_host(&document.hosts[index]))
                    .unwrap_or_default();
                let count = document.hosts.len();

                Self {
                    path,
                    document,
                    selected,
                    editor,
                    search: String::new(),
                    dirty: false,
                    confirm_delete: false,
                    status: StatusMessage::success(format!("{count} host(s) carregado(s).")),
                }
            }
            Err(error) => Self {
                path,
                document: ConfigDocument::default(),
                selected: None,
                editor: HostEditor::default(),
                search: String::new(),
                dirty: false,
                confirm_delete: false,
                status: StatusMessage::error(error.to_string()),
            },
        }
    }

    // Verifica se o formulário diverge do Host armazenado no documento.
    fn editor_has_changes(&self) -> bool {
        self.selected
            .and_then(|index| self.document.hosts.get(index))
            .is_some_and(|host| self.editor != HostEditor::from_host(host))
    }

    // Seleciona um item da lista, protegendo alterações ainda não aplicadas.
    fn select_host(&mut self, index: usize) {
        if self.editor_has_changes() {
            self.status = StatusMessage::warning(
                "Aplique ou desfaça as alterações do formulário antes de trocar de host.",
            );
            return;
        }

        self.selected = Some(index);
        self.editor = HostEditor::from_host(&self.document.hosts[index]);
        self.status = StatusMessage::information("Host selecionado.");
    }

    // Valida e aplica o formulário ao Host selecionado.
    fn apply_editor(&mut self) {
        if let Err(message) = self.editor.validate() {
            self.status = StatusMessage::error(message);
            return;
        }

        let Some(index) = self.selected else {
            return;
        };

        self.editor.apply_to(&mut self.document.hosts[index]);
        self.dirty = true;
        self.status = StatusMessage::success(
            "Alterações aplicadas em memória. Clique em Salvar para gravar o arquivo.",
        );
    }

    // Adiciona um bloco e já o seleciona para edição.
    fn add_host(&mut self) {
        if self.editor_has_changes() {
            self.status = StatusMessage::warning(
                "Aplique ou desfaça o formulário atual antes de criar outro host.",
            );
            return;
        }

        self.document.hosts.push(HostEntry::new());
        let index = self.document.hosts.len() - 1;
        self.selected = Some(index);
        self.editor = HostEditor::from_host(&self.document.hosts[index]);
        self.dirty = true;
        self.status = StatusMessage::information("Novo host criado em memória.");
    }

    // Remove o item somente depois da confirmação visual.
    fn delete_selected(&mut self) {
        let Some(index) = self.selected else {
            return;
        };

        self.document.hosts.remove(index);
        self.selected = if self.document.hosts.is_empty() {
            None
        } else {
            Some(index.min(self.document.hosts.len() - 1))
        };
        self.editor = self
            .selected
            .map(|new_index| HostEditor::from_host(&self.document.hosts[new_index]))
            .unwrap_or_default();
        self.dirty = true;
        self.confirm_delete = false;
        self.status = StatusMessage::warning("Host removido em memória. Salve para confirmar.");
    }

    // Reabre o arquivo. O botão fica desabilitado se houver alterações locais.
    fn reload(&mut self) {
        match read_document(&self.path) {
            Ok(document) => {
                self.document = document;
                self.selected = (!self.document.hosts.is_empty()).then_some(0);
                self.editor = self
                    .selected
                    .map(|index| HostEditor::from_host(&self.document.hosts[index]))
                    .unwrap_or_default();
                self.dirty = false;
                self.status = StatusMessage::success("Arquivo recarregado.");
            }
            Err(error) => self.status = StatusMessage::error(error.to_string()),
        }
    }

    // Abre o seletor nativo e troca de arquivo sem descartar edições pendentes.
    fn open_file(&mut self) {
        if self.dirty || self.editor_has_changes() {
            self.status = StatusMessage::warning(
                "Salve ou desfaça as alterações antes de abrir outro arquivo.",
            );
            return;
        }

        let mut dialog = rfd::FileDialog::new();
        if let Some(directory) = self.path.parent().filter(|path| path.is_dir()) {
            dialog = dialog.set_directory(directory);
        }

        let Some(path) = dialog.pick_file() else {
            return;
        };

        match read_document(&path) {
            Ok(document) => {
                let count = document.hosts.len();
                self.path = path;
                self.document = document;
                self.selected = (!self.document.hosts.is_empty()).then_some(0);
                self.editor = self
                    .selected
                    .map(|index| HostEditor::from_host(&self.document.hosts[index]))
                    .unwrap_or_default();
                self.search.clear();
                self.dirty = false;
                self.confirm_delete = false;
                self.status = StatusMessage::success(format!(
                    "Arquivo aberto. {count} host(s) carregado(s)."
                ));
            }
            Err(error) => self.status = StatusMessage::error(error.to_string()),
        }
    }

    // Valida o documento, cria backup e salva.
    fn save(&mut self) {
        if self.editor_has_changes() {
            self.status = StatusMessage::warning(
                "Existem campos não aplicados. Clique em Aplicar alterações primeiro.",
            );
            return;
        }

        if self.document.contains_match {
            self.status = StatusMessage::error(
                "O arquivo contém Match. Por segurança, esta versão permite visualizar, mas não salvar.",
            );
            return;
        }

        if let Err(message) = validate_unique_hosts(&self.document) {
            self.status = StatusMessage::error(message);
            return;
        }

        match write_document(&self.path, &self.document) {
            Ok(backup) => {
                self.dirty = false;
                self.status =
                    StatusMessage::success(format!("Arquivo salvo. Backup: {}", backup.display()));
            }
            Err(error) => self.status = StatusMessage::error(error.to_string()),
        }
    }

    // Barra superior com as ações que afetam o documento inteiro.
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .fill(surface())
            .stroke(egui::Stroke::new(1.0, border()))
            .corner_radius(12)
            .inner_margin(egui::Margin::symmetric(22, 16))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("SSH CONFIG MANAGER")
                                .size(20.0)
                                .strong()
                                .color(text_primary()),
                        );
                        ui.label(
                            egui::RichText::new("Gerencie conexões com segurança")
                                .size(12.0)
                                .color(text_secondary()),
                        );
                    });

                    // Coloca as ações do documento alinhadas à direita.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let save_button = egui::Button::new(
                            egui::RichText::new("Salvar arquivo")
                                .strong()
                                .color(egui::Color32::WHITE),
                        )
                        .fill(accent())
                        .corner_radius(7)
                        .min_size(egui::vec2(120.0, 36.0));

                        if ui.add_enabled(self.dirty, save_button).clicked() {
                            self.save();
                        }

                        if ui
                            .add_enabled(
                                !self.dirty,
                                egui::Button::new("Recarregar")
                                    .corner_radius(7)
                                    .min_size(egui::vec2(92.0, 36.0)),
                            )
                            .on_disabled_hover_text(
                                "Salve as alterações antes de recarregar o arquivo.",
                            )
                            .clicked()
                        {
                            self.reload();
                        }

                        if ui
                            .add(
                                egui::Button::new("+  Novo host")
                                    .corner_radius(7)
                                    .min_size(egui::vec2(108.0, 36.0)),
                            )
                            .clicked()
                        {
                            self.add_host();
                        }

                        if ui
                            .add_enabled(
                                !self.dirty && !self.editor_has_changes(),
                                egui::Button::new("Abrir arquivo…")
                                    .corner_radius(7)
                                    .min_size(egui::vec2(108.0, 36.0)),
                            )
                            .on_disabled_hover_text(
                                "Salve ou desfaça as alterações antes de abrir outro arquivo.",
                            )
                            .clicked()
                        {
                            self.open_file();
                        }
                    });
                });

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("ARQUIVO")
                            .size(10.0)
                            .strong()
                            .color(text_muted()),
                    );
                    egui::Frame::new()
                        .fill(background())
                        .corner_radius(6)
                        .inner_margin(egui::Margin::symmetric(9, 5))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(self.path.display().to_string())
                                    .monospace()
                                    .size(11.0)
                                    .color(text_secondary()),
                            );
                        });

                    let (label, color) = if self.dirty {
                        ("ALTERADO", warning())
                    } else {
                        ("SINCRONIZADO", success())
                    };
                    ui.label(
                        egui::RichText::new(format!("● {label}"))
                            .size(10.0)
                            .strong()
                            .color(color),
                    );
                });
            });
    }

    // Coluna esquerda com os aliases encontrados no arquivo.
    fn host_list(&mut self, ui: &mut egui::Ui) {
        ui.set_width(278.0);
        egui::Frame::new()
            .fill(surface())
            .stroke(egui::Stroke::new(1.0, border()))
            .corner_radius(12)
            .inner_margin(egui::Margin::same(16))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Seus hosts").size(16.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        egui::Frame::new()
                            .fill(accent_soft())
                            .corner_radius(10)
                            .inner_margin(egui::Margin::symmetric(8, 3))
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new(self.document.hosts.len().to_string())
                                        .size(11.0)
                                        .strong()
                                        .color(accent_light()),
                                );
                            });
                    });
                });
                ui.add_space(12.0);

                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Buscar host, domínio ou usuário...")
                        .desired_width(f32::INFINITY)
                        .margin(egui::Margin::symmetric(10, 8)),
                );
                ui.add_space(10.0);

                let query = self.search.trim().to_lowercase();
                let mut clicked_index = None;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(465.0)
                    .show(ui, |ui| {
                        for (index, host) in self.document.hosts.iter().enumerate() {
                            let hostname = host.option("HostName");
                            let user = host.option("User");
                            let searchable =
                                format!("{} {hostname} {user}", host.names).to_lowercase();
                            if !query.is_empty() && !searchable.contains(&query) {
                                continue;
                            }

                            let selected = self.selected == Some(index);
                            let subtitle = if hostname.is_empty() {
                                "HostName não definido".to_owned()
                            } else if user.is_empty() {
                                hostname
                            } else {
                                format!("{user}@{hostname}")
                            };
                            let label =
                                egui::RichText::new(format!("{}\n{}", host.names, subtitle))
                                    .size(13.0)
                                    .color(if selected {
                                        egui::Color32::WHITE
                                    } else {
                                        text_primary()
                                    });

                            let button = egui::Button::new(label)
                                .selected(selected)
                                .fill(if selected { accent() } else { surface_raised() })
                                .stroke(egui::Stroke::new(
                                    1.0,
                                    if selected { accent_light() } else { border() },
                                ))
                                .corner_radius(8);

                            if ui.add_sized([ui.available_width(), 54.0], button).clicked() {
                                clicked_index = Some(index);
                            }
                            ui.add_space(6.0);
                        }

                        if self.document.hosts.is_empty() {
                            ui.add_space(30.0);
                            ui.vertical_centered(|ui| {
                                ui.label(egui::RichText::new("Nenhum host").strong());
                                ui.label(
                                    egui::RichText::new(
                                        "Crie sua primeira conexão no botão acima.",
                                    )
                                    .size(11.0)
                                    .color(text_muted()),
                                );
                            });
                        }
                    });

                if let Some(index) = clicked_index {
                    self.select_host(index);
                }
            });
    }

    // Coluna direita com os campos editáveis.
    fn host_form(&mut self, ui: &mut egui::Ui) {
        let Some(index) = self.selected else {
            egui::Frame::new()
                .fill(surface())
                .stroke(egui::Stroke::new(1.0, border()))
                .corner_radius(12)
                .inner_margin(egui::Margin::same(24))
                .show(ui, |ui| {
                    ui.set_min_height(470.0);
                    ui.centered_and_justified(|ui| {
                        ui.vertical_centered(|ui| {
                            ui.label(
                                egui::RichText::new("Nenhum host selecionado")
                                    .size(18.0)
                                    .strong(),
                            );
                            ui.label(
                                egui::RichText::new(
                                    "Selecione um item da lista ou crie uma nova conexão.",
                                )
                                .color(text_secondary()),
                            );
                        });
                    });
                });
            return;
        };

        egui::Frame::new()
            .fill(surface())
            .stroke(egui::Stroke::new(1.0, border()))
            .corner_radius(12)
            .inner_margin(egui::Margin::same(22))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(&self.editor.names).size(19.0).strong());
                        ui.label(
                            egui::RichText::new(format!(
                                "Configuração do bloco Host {}",
                                index + 1
                            ))
                            .size(11.0)
                            .color(text_muted()),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let state = if self.editor_has_changes() {
                            egui::RichText::new("EDIÇÃO PENDENTE").color(warning())
                        } else {
                            egui::RichText::new("ATUALIZADO").color(success())
                        };
                        ui.label(state.size(10.0).strong());
                    });
                });

                ui.add_space(18.0);
                ui.label(
                    egui::RichText::new("CONEXÃO")
                        .size(10.0)
                        .strong()
                        .color(accent_light()),
                );
                ui.add_space(7.0);

                egui::Grid::new("connection_form")
                    .num_columns(2)
                    .min_col_width(110.0)
                    .spacing([18.0, 12.0])
                    .show(ui, |ui| {
                        field(
                            ui,
                            "Alias do host",
                            "Usado em: ssh meu-servidor",
                            &mut self.editor.names,
                        );
                        field(
                            ui,
                            "Endereço",
                            "IP ou domínio verdadeiro",
                            &mut self.editor.host_name,
                        );
                        field(
                            ui,
                            "Usuário",
                            "Usuário da máquina remota",
                            &mut self.editor.user,
                        );
                        field(
                            ui,
                            "Porta",
                            "A porta SSH padrão é 22",
                            &mut self.editor.port,
                        );
                    });

                ui.add_space(18.0);
                ui.separator();
                ui.add_space(14.0);
                ui.label(
                    egui::RichText::new("AUTENTICAÇÃO")
                        .size(10.0)
                        .strong()
                        .color(accent_light()),
                );
                ui.add_space(7.0);
                egui::Grid::new("authentication_form")
                    .num_columns(2)
                    .min_col_width(110.0)
                    .spacing([18.0, 12.0])
                    .show(ui, |ui| {
                        field(
                            ui,
                            "Chave privada",
                            "Exemplo: ~/.ssh/id_ed25519",
                            &mut self.editor.identity_file,
                        );
                    });

                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    let apply = egui::Button::new(
                        egui::RichText::new("Aplicar alterações")
                            .strong()
                            .color(egui::Color32::WHITE),
                    )
                    .fill(accent())
                    .corner_radius(7)
                    .min_size(egui::vec2(150.0, 36.0));
                    if ui.add_enabled(self.editor_has_changes(), apply).clicked() {
                        self.apply_editor();
                    }

                    if ui
                        .add_enabled(
                            self.editor_has_changes(),
                            egui::Button::new("Desfazer")
                                .corner_radius(7)
                                .min_size(egui::vec2(84.0, 36.0)),
                        )
                        .clicked()
                    {
                        self.editor = HostEditor::from_host(&self.document.hosts[index]);
                        self.status = StatusMessage::information("Formulário restaurado.");
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("Excluir host").color(danger()),
                                )
                                .corner_radius(7)
                                .min_size(egui::vec2(100.0, 36.0)),
                            )
                            .clicked()
                        {
                            self.confirm_delete = true;
                        }
                    });
                });
            });

        ui.add_space(12.0);
        egui::Frame::new()
            .fill(surface())
            .stroke(egui::Stroke::new(1.0, border()))
            .corner_radius(12)
            .inner_margin(egui::Margin::symmetric(18, 12))
            .show(ui, |ui| {
                ui.collapsing(
                    egui::RichText::new("Visualizar configuração completa").strong(),
                    |ui| {
                        ui.add_space(8.0);
                        let mut preview = serialize_document(&self.document);
                        ui.add(
                            egui::TextEdit::multiline(&mut preview)
                                .font(egui::TextStyle::Monospace)
                                .desired_rows(10)
                                .desired_width(f32::INFINITY)
                                .interactive(false),
                        );
                    },
                );
            });
    }

    // Rodapé com mensagens de erro, aviso ou sucesso.
    fn status_bar(&self, ui: &mut egui::Ui) {
        let (color, fill, title) = match self.status.kind {
            StatusKind::Information => (accent_light(), accent_soft(), "INFORMAÇÃO"),
            StatusKind::Success => (success(), egui::Color32::from_rgb(19, 55, 45), "SUCESSO"),
            StatusKind::Warning => (warning(), egui::Color32::from_rgb(63, 47, 21), "ATENÇÃO"),
            StatusKind::Error => (danger(), egui::Color32::from_rgb(64, 29, 34), "ERRO"),
        };
        egui::Frame::new()
            .fill(fill)
            .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.55)))
            .corner_radius(8)
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(title).size(10.0).strong().color(color));
                    ui.separator();
                    ui.label(
                        egui::RichText::new(&self.status.text)
                            .size(12.0)
                            .color(text_primary()),
                    );
                });
            });
    }

    // Janela modal simples para impedir exclusões acidentais.
    fn delete_dialog(&mut self, context: &egui::Context) {
        if !self.confirm_delete {
            return;
        }

        egui::Window::new("Confirmar exclusão")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(context, |ui| {
                ui.label(format!("Excluir o Host '{}'?", self.editor.names));
                ui.label("A remoção só chega ao arquivo quando você salvar.");
                ui.horizontal(|ui| {
                    if ui.button("Cancelar").clicked() {
                        self.confirm_delete = false;
                    }
                    if ui.button("Excluir").clicked() {
                        self.delete_selected();
                    }
                });
            });
    }
}

// `eframe::App` é o contrato que transforma nossa struct em aplicação gráfica.
impl eframe::App for ManagerApp {
    // `ui` é chamada muitas vezes por segundo enquanto a janela está visível.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(background())
                    .inner_margin(egui::Margin::same(18)),
            )
            .show(ui, |ui| {
                self.top_bar(ui);
                ui.add_space(14.0);

                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| self.host_list(ui));
                    ui.add_space(14.0);
                    ui.vertical(|ui| {
                        ui.set_min_width(ui.available_width());
                        self.host_form(ui);
                    });
                });

                ui.add_space(12.0);
                self.status_bar(ui);
            });

        self.delete_dialog(&context);
    }
}

// Desenha uma linha do formulário e mostra uma explicação ao parar o mouse.
fn field(ui: &mut egui::Ui, label: &str, hint: &str, value: &mut String) {
    ui.vertical(|ui| {
        ui.label(
            egui::RichText::new(label)
                .size(12.0)
                .strong()
                .color(text_secondary()),
        );
        ui.label(egui::RichText::new(hint).size(10.0).color(text_muted()));
    });
    ui.add(
        egui::TextEdit::singleline(value)
            .desired_width(430.0)
            .margin(egui::Margin::symmetric(10, 8)),
    )
    .on_hover_text(hint);
    ui.end_row();
}

// Paleta centralizada: evita espalhar valores RGB pela interface.
fn background() -> egui::Color32 {
    egui::Color32::from_rgb(12, 17, 27)
}

fn surface() -> egui::Color32 {
    egui::Color32::from_rgb(20, 27, 40)
}

fn surface_raised() -> egui::Color32 {
    egui::Color32::from_rgb(27, 36, 52)
}

fn border() -> egui::Color32 {
    egui::Color32::from_rgb(45, 56, 75)
}

fn accent() -> egui::Color32 {
    egui::Color32::from_rgb(55, 105, 235)
}

fn accent_light() -> egui::Color32 {
    egui::Color32::from_rgb(112, 154, 255)
}

fn accent_soft() -> egui::Color32 {
    egui::Color32::from_rgb(28, 48, 88)
}

fn text_primary() -> egui::Color32 {
    egui::Color32::from_rgb(235, 240, 249)
}

fn text_secondary() -> egui::Color32 {
    egui::Color32::from_rgb(169, 181, 201)
}

fn text_muted() -> egui::Color32 {
    egui::Color32::from_rgb(112, 126, 150)
}

fn success() -> egui::Color32 {
    egui::Color32::from_rgb(74, 222, 160)
}

fn warning() -> egui::Color32 {
    egui::Color32::from_rgb(251, 191, 74)
}

fn danger() -> egui::Color32 {
    egui::Color32::from_rgb(248, 113, 113)
}

// Define tipografia, espaçamento e cores globais uma única vez na inicialização.
fn configure_style(context: &egui::Context) {
    // O egui 0.36 mantém estilos separados para os temas claro e escuro.
    context.set_theme(egui::Theme::Dark);
    let mut style = (*context.style_of(egui::Theme::Dark)).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.interact_size.y = 34.0;

    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::new(20.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::new(13.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::new(12.5, egui::FontFamily::Proportional),
    );

    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = background();
    style.visuals.window_fill = surface();
    style.visuals.extreme_bg_color = egui::Color32::from_rgb(14, 20, 31);
    style.visuals.faint_bg_color = surface_raised();
    style.visuals.selection.bg_fill = accent();
    style.visuals.selection.stroke = egui::Stroke::new(1.0, accent_light());
    style.visuals.widgets.noninteractive.bg_fill = surface();
    style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, border());
    style.visuals.widgets.inactive.bg_fill = surface_raised();
    style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, border());
    style.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(38, 50, 72);
    style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, accent_light());
    style.visuals.widgets.active.bg_fill = accent();
    style.visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, accent_light());
    style.visuals.override_text_color = Some(text_primary());
    style.visuals.window_corner_radius = egui::CornerRadius::same(12);

    context.set_style_of(egui::Theme::Dark, style);
}

// Lê e converte o arquivo de texto em nosso modelo.
fn read_document(path: &Path) -> Result<ConfigDocument, io::Error> {
    let content = fs::read_to_string(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("Não foi possível abrir {}: {error}", path.display()),
        )
    })?;
    Ok(parse_document(&content))
}

// Parser conservador: mantém as linhas internas exatamente como foram lidas.
fn parse_document(content: &str) -> ConfigDocument {
    let mut document = ConfigDocument::default();
    let mut current: Option<HostEntry> = None;

    for original_line in content.lines() {
        if let Some((key, value)) = split_directive(original_line) {
            if key.eq_ignore_ascii_case("match") {
                document.contains_match = true;
            }

            if key.eq_ignore_ascii_case("host") {
                if let Some(previous) = current.take() {
                    document.hosts.push(previous);
                }
                current = Some(HostEntry {
                    names: value.to_owned(),
                    body: Vec::new(),
                });
                continue;
            }
        }

        if let Some(host) = &mut current {
            host.body.push(original_line.to_owned());
        } else {
            document.prefix.push(original_line.to_owned());
        }
    }

    if let Some(last) = current {
        document.hosts.push(last);
    }

    document
}

// Retorna `(chave, valor)` para sintaxes `User ubuntu` e `User=ubuntu`.
fn split_directive(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    if let Some((key, value)) = line.split_once('=') {
        return Some((key.trim(), value.trim()));
    }

    let key_end = line.find(char::is_whitespace).unwrap_or(line.len());
    let (key, value) = line.split_at(key_end);
    Some((key, value.trim()))
}

// Gera o texto que será mostrado na prévia e salvo no disco.
fn serialize_document(document: &ConfigDocument) -> String {
    let mut output = String::new();

    for line in &document.prefix {
        output.push_str(line);
        output.push('\n');
    }

    for host in &document.hosts {
        // Deixa uma única separação visual antes do próximo Host.
        if !output.is_empty() && !output.ends_with("\n\n") {
            output.push('\n');
        }
        output.push_str("Host ");
        output.push_str(host.names.trim());
        output.push('\n');
        for line in &host.body {
            output.push_str(line);
            output.push('\n');
        }
    }

    output
}

// Impede aliases idênticos, que poderiam deixar o resultado ambíguo.
fn validate_unique_hosts(document: &ConfigDocument) -> Result<(), String> {
    let mut occurrences: HashMap<&str, usize> = HashMap::new();

    for host in &document.hosts {
        for name in host.names.split_whitespace() {
            if name.contains(['*', '?', '!']) {
                continue;
            }
            *occurrences.entry(name).or_default() += 1;
        }
    }

    if let Some((name, _)) = occurrences.iter().find(|(_, count)| **count > 1) {
        return Err(format!(
            "O alias '{name}' aparece em mais de um bloco Host."
        ));
    }

    Ok(())
}

// Salva somente depois de copiar o conteúdo anterior para `.bak`.
fn write_document(path: &Path, document: &ConfigDocument) -> Result<PathBuf, io::Error> {
    let mut backup_name = path.as_os_str().to_owned();
    backup_name.push(".bak");
    let backup = PathBuf::from(backup_name);

    fs::copy(path, &backup)?;
    fs::write(path, serialize_document(document))?;
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hosts_and_known_options() {
        let text = "# exemplo\nHost github\n    HostName github.com\n    User git\n\nHost server\n    Port 2222\n";
        let document = parse_document(text);

        assert_eq!(document.prefix, ["# exemplo"]);
        assert_eq!(document.hosts.len(), 2);
        assert_eq!(document.hosts[0].names, "github");
        assert_eq!(document.hosts[0].option("hostname"), "github.com");
        assert_eq!(document.hosts[1].option("Port"), "2222");
    }

    #[test]
    fn editing_preserves_unknown_directives_and_comments() {
        let mut host = HostEntry {
            names: "server".to_owned(),
            body: vec![
                "    # comentário importante".to_owned(),
                "    ForwardAgent yes".to_owned(),
                "    User old".to_owned(),
            ],
        };

        host.set_option("User", "new");

        assert!(host.body.iter().any(|line| line.contains("comentário")));
        assert!(host.body.iter().any(|line| line.contains("ForwardAgent")));
        assert_eq!(host.option("User"), "new");
    }

    #[test]
    fn rejects_invalid_port_and_duplicate_alias() {
        let editor = HostEditor {
            names: "server".to_owned(),
            port: "70000".to_owned(),
            ..Default::default()
        };
        assert!(editor.validate().is_err());

        let document = parse_document("Host same\nHost same\n");
        assert!(validate_unique_hosts(&document).is_err());
    }
}
