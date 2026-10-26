//! A configuração dos remotos do rclone: a lista, os provedores (com as opções de cada um) e a
//! criação não interativa, que devolve as perguntas do rclone uma de cada vez.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use super::{Rcd, RcloneError};

/// Um remoto já configurado.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteInfo {
    /// O nome (`gdrive`).
    pub name: String,
    /// O tipo (`drive`, `s3`, `webdav`…).
    pub kind: String,
}

/// Um exemplo de valor de uma opção.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Example {
    /// O valor.
    pub value: String,
    /// A explicação.
    pub help: String,
    /// Os provedores a que o exemplo se aplica (vazio: todos).
    pub provider: String,
}

/// Uma opção de um provedor (ou a pergunta da configuração).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProviderOption {
    /// O nome (`client_id`).
    pub name: String,
    /// O texto de ajuda (várias linhas).
    pub help: String,
    /// O valor padrão, em texto.
    pub default: String,
    /// Os exemplos.
    pub examples: Vec<Example>,
    /// `true` se só os exemplos valem.
    pub exclusive: bool,
    /// `true` se a opção é obrigatória.
    pub required: bool,
    /// `true` se é uma senha.
    pub is_password: bool,
    /// `true` se é uma opção avançada.
    pub advanced: bool,
    /// Onde a opção se esconde (bits do rclone: 1 linha de comando, 2 configurador).
    pub hide: u32,
    /// O tipo (`string`, `bool`, `int`, `SizeSuffix`, `Duration`…).
    pub kind: String,
    /// Os provedores a que a opção se aplica (`AWS`, `!AWS,Minio`; vazio: todos).
    pub provider: String,
}

impl ProviderOption {
    /// `true` se a opção não aparece no formulário.
    pub fn hidden(&self) -> bool {
        self.hide & 2 != 0
    }

    /// `true` se a opção vale para o `provider` escolhido (`""` se não há escolha ainda).
    pub fn applies_to(&self, provider: &str) -> bool {
        applies(&self.provider, provider)
    }

    fn from_json(value: &Value) -> ProviderOption {
        let texto = |nome: &str| {
            value
                .get(nome)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        let marca = |nome: &str| value.get(nome).and_then(Value::as_bool).unwrap_or(false);
        let padrao = value
            .get("DefaultStr")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| match value.get("Default") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Null) | None => String::new(),
                Some(outro) => outro.to_string(),
            });
        let examples = value
            .get("Examples")
            .and_then(Value::as_array)
            .map(|lista| {
                lista
                    .iter()
                    .map(|e| Example {
                        value: e
                            .get("Value")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                        help: e
                            .get("Help")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                        provider: e
                            .get("Provider")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        ProviderOption {
            name: texto("Name"),
            help: texto("Help"),
            default: padrao,
            examples,
            exclusive: marca("Exclusive"),
            required: marca("Required"),
            is_password: marca("IsPassword"),
            advanced: marca("Advanced"),
            hide: value.get("Hide").and_then(Value::as_u64).unwrap_or(0) as u32,
            kind: texto("Type"),
            provider: texto("Provider"),
        }
    }
}

/// `true` se a regra `regra` (`AWS`, `!AWS,Minio`) vale para `atual`.
pub fn applies(regra: &str, atual: &str) -> bool {
    let regra = regra.trim();
    if regra.is_empty() {
        return true;
    }
    let (nega, lista) = match regra.strip_prefix('!') {
        Some(resto) => (true, resto),
        None => (false, regra),
    };
    let contem = lista.split(',').any(|nome| nome.trim() == atual);
    contem != nega
}

/// Um provedor (um tipo de remoto) do rclone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Provider {
    /// O nome do tipo (`drive`).
    pub name: String,
    /// A descrição (`Google Drive`).
    pub description: String,
    /// As opções, na ordem do rclone.
    pub options: Vec<ProviderOption>,
}

/// O que a criação (ou a atualização) de um remoto devolveu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigStep {
    /// O remoto está configurado.
    Done,
    /// O rclone precisa de uma resposta para seguir.
    Question {
        /// O estado a devolver em [`Rcd::config_answer`].
        state: String,
        /// A pergunta.
        option: ProviderOption,
        /// O erro da resposta anterior, se ela foi recusada.
        error: String,
    },
}

fn passo(resposta: &Value) -> ConfigStep {
    let estado = resposta
        .get("State")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    match resposta.get("Option") {
        Some(opcao) if !opcao.is_null() && !estado.is_empty() => ConfigStep::Question {
            state: estado,
            option: ProviderOption::from_json(opcao),
            error: resposta
                .get("Error")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        },
        _ => ConfigStep::Done,
    }
}

fn parametros(valores: &[(String, String)]) -> Value {
    Value::Object(
        valores
            .iter()
            .map(|(nome, valor)| (nome.clone(), Value::String(valor.clone())))
            .collect::<Map<_, _>>(),
    )
}

impl Rcd {
    /// Os remotos configurados, com o tipo de cada um (os valores não vêm).
    pub fn config_remotes(&self) -> Result<Vec<RemoteInfo>, RcloneError> {
        let nomes = self.call("config/listremotes", json!({}))?;
        let tudo = self.call("config/dump", json!({}))?;
        Ok(nomes
            .get("remotes")
            .and_then(Value::as_array)
            .map(|lista| {
                lista
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|nome| RemoteInfo {
                        name: nome.to_owned(),
                        kind: tudo
                            .get(nome)
                            .and_then(|v| v.get("type"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Os provedores e as opções de cada um.
    pub fn config_providers(&self) -> Result<Vec<Provider>, RcloneError> {
        let resposta = self.call("config/providers", json!({}))?;
        Ok(resposta
            .get("providers")
            .and_then(Value::as_array)
            .map(|lista| {
                lista
                    .iter()
                    .map(|p| Provider {
                        name: p
                            .get("Name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                        description: p
                            .get("Description")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                        options: p
                            .get("Options")
                            .and_then(Value::as_array)
                            .map(|o| o.iter().map(ProviderOption::from_json).collect())
                            .unwrap_or_default(),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Cria o remoto `name` do tipo `kind`. As senhas vão em claro (o rclone as ofusca). Se o
    /// rclone precisar de mais respostas, devolve a primeira pergunta.
    pub fn config_create(
        &self,
        name: &str,
        kind: &str,
        params: &[(String, String)],
    ) -> Result<ConfigStep, RcloneError> {
        let resposta = self.call_login(
            "config/create",
            json!({
                "name": name,
                "type": kind,
                "parameters": parametros(params),
                "opt": { "nonInteractive": true, "obscure": true },
            }),
        )?;
        Ok(passo(&resposta))
    }

    /// Responde à pergunta de um [`ConfigStep::Question`]. A resposta `true` ao
    /// `config_is_local` abre o navegador e só volta quando o login termina (até 10 minutos).
    pub fn config_answer(
        &self,
        name: &str,
        state: &str,
        result: &str,
    ) -> Result<ConfigStep, RcloneError> {
        let resposta = self.call_login(
            "config/update",
            json!({
                "name": name,
                "parameters": {},
                "opt": {
                    "nonInteractive": true,
                    "continue": true,
                    "state": state,
                    "result": result,
                },
            }),
        )?;
        Ok(passo(&resposta))
    }

    /// Os valores do remoto (as senhas vêm como o rclone as guarda: ofuscadas).
    pub fn config_get(&self, name: &str) -> Result<BTreeMap<String, String>, RcloneError> {
        let resposta = self.call("config/get", json!({ "name": name }))?;
        Ok(resposta
            .as_object()
            .map(|mapa| {
                mapa.iter()
                    .map(|(chave, valor)| {
                        let texto = match valor {
                            Value::String(s) => s.clone(),
                            outro => outro.to_string(),
                        };
                        (chave.clone(), texto)
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Muda os valores de `params` no remoto `name`. As senhas vão em claro.
    pub fn config_update(
        &self,
        name: &str,
        params: &[(String, String)],
    ) -> Result<ConfigStep, RcloneError> {
        let resposta = self.call_login(
            "config/update",
            json!({
                "name": name,
                "parameters": parametros(params),
                "opt": { "nonInteractive": true, "obscure": true },
            }),
        )?;
        Ok(passo(&resposta))
    }

    /// Apaga o remoto.
    pub fn config_delete(&self, name: &str) -> Result<(), RcloneError> {
        self.call("config/delete", json!({ "name": name }))
            .map(drop)
    }
}
