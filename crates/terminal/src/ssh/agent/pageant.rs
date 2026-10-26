//! O que é do Pageant (o agente do PuTTY) e não depende do Windows: o nome do pipe e a troca de
//! mensagens pelo mapa de memória do `WM_COPYDATA`. Fica aqui, testado em qualquer sistema; as
//! chamadas do Windows (o `CryptProtectMemory`, o nome do usuário, a janela, o pipe com dono) são
//! do crate `tomni-ui-win32`.
//!
//! Tudo conferido no código do PuTTY 0.81 (`windows/utils/agent_named_pipe_name.c`,
//! `windows/utils/cryptoapi.c` e `windows/pageant.c`).

use sha2::{Digest, Sha256};

/// O `dwData` do `WM_COPYDATA` do Pageant (`AGENT_COPYDATA_ID`).
pub const AGENT_COPYDATA_ID: usize = 0x804e_50ba;

/// A maior mensagem que o Pageant atende (`AGENT_MAX_MSGLEN`), com o tamanho na frente.
pub const AGENT_MAX_MSGLEN: usize = 262_144;

/// A resposta `SSH_AGENT_FAILURE` com a moldura: `00 00 00 01 05`.
const FAILURE_FRAME: [u8; 5] = [0, 0, 0, 1, 5];

/// O que o PuTTY passa pelo `CryptProtectMemory` para tirar o sufixo do nome do pipe: a palavra
/// "Pageant" com o `\0`, completada com zeros até 16 bytes (o bloco do `CryptProtectMemory`).
pub fn pageant_obfuscation_input() -> [u8; 16] {
    let mut buffer = [0u8; 16];
    buffer[..8].copy_from_slice(b"Pageant\0");
    buffer
}

/// O sufixo do nome do pipe: o SHA-256 do `put_string` de `protegido` (o tamanho em quatro bytes
/// e o conteúdo), em 64 caracteres hexadecimais minúsculos. `protegido` é o
/// [`pageant_obfuscation_input`] depois do `CryptProtectMemory` (ou ele mesmo, se a chamada
/// falhar: o PuTTY segue assim).
pub fn pageant_pipe_suffix(protegido: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update((protegido.len() as u32).to_be_bytes());
    hash.update(protegido);
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// O nome do pipe do Pageant: `\\.\pipe\pageant.<usuário>.<sufixo>`. O usuário é cortado no
/// primeiro `@` (o PuTTY usa o nome principal do usuário e tira o domínio).
pub fn pageant_pipe_name(usuario: &str, sufixo: &str) -> String {
    let usuario = usuario.split('@').next().unwrap_or(usuario);
    format!(r"\\.\pipe\pageant.{usuario}.{sufixo}")
}

/// O miolo do `WM_COPYDATA`, sem ponteiro nenhum: `mapa` é a memória que o cliente mapeou, com a
/// mensagem do agente (`u32` com o tamanho e o corpo). Valida, chama `responder` com o corpo e
/// grava a resposta com o tamanho na frente, no mesmo mapa.
///
/// Devolve falso, sem chamar o `responder`, quando o mapa tem menos de 5 bytes ou o tamanho
/// declarado passa de [`AGENT_MAX_MSGLEN`] ou do que o mapa guarda. Uma resposta que não cabe
/// vira `SSH_AGENT_FAILURE`.
pub fn copydata_exchange(mapa: &mut [u8], responder: impl FnOnce(&[u8]) -> Vec<u8>) -> bool {
    let Some((tamanho, _)) = mapa.split_first_chunk::<4>() else {
        return false;
    };
    if mapa.len() < 5 {
        return false;
    }
    let tamanho = u32::from_be_bytes(*tamanho) as usize;
    if tamanho > AGENT_MAX_MSGLEN || tamanho > mapa.len() - 4 {
        return false;
    }
    let resposta = responder(&mapa[4..4 + tamanho]);
    if resposta.len() > mapa.len() - 4 {
        mapa[..FAILURE_FRAME.len()].copy_from_slice(&FAILURE_FRAME);
    } else {
        mapa[..4].copy_from_slice(&(resposta.len() as u32).to_be_bytes());
        mapa[4..4 + resposta.len()].copy_from_slice(&resposta);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sufixo_do_pipe_sem_cifra() {
        // O SHA-256 de `00 00 00 10` + "Pageant\0" + 8 zeros, conferido de fora com
        // `printf '\0\0\0\x10Pageant\0\0\0\0\0\0\0\0\0' | sha256sum`.
        assert_eq!(
            pageant_pipe_suffix(&pageant_obfuscation_input()),
            "2e9e7389b30cd8240bf15518e9a228d6537dc38d64b2070a189ca2fe5452d492"
        );
    }

    #[test]
    fn a_entrada_da_ofuscacao_tem_um_bloco_com_pageant_e_zeros() {
        let entrada = pageant_obfuscation_input();
        assert_eq!(&entrada[..8], b"Pageant\0");
        assert!(entrada[8..].iter().all(|&b| b == 0));
    }

    #[test]
    fn nome_do_pipe_corta_o_usuario_no_arroba() {
        assert_eq!(
            pageant_pipe_name("ana@empresa.com", "abc123"),
            r"\\.\pipe\pageant.ana.abc123"
        );
        assert_eq!(pageant_pipe_name("ana", "abc"), r"\\.\pipe\pageant.ana.abc");
    }

    fn mapa_com(corpo: &[u8], tamanho: usize) -> Vec<u8> {
        let mut mapa = vec![0u8; tamanho];
        mapa[..4].copy_from_slice(&(corpo.len() as u32).to_be_bytes());
        mapa[4..4 + corpo.len()].copy_from_slice(corpo);
        mapa
    }

    #[test]
    fn uma_mensagem_valida_chama_o_responder_e_grava_a_resposta_com_o_tamanho() {
        let mut mapa = mapa_com(&[11], 64);
        let mut recebido = Vec::new();
        let respondeu = copydata_exchange(&mut mapa, |corpo| {
            recebido = corpo.to_vec();
            vec![12, 0, 0, 0, 0]
        });
        assert!(respondeu);
        assert_eq!(recebido, [11]);
        assert_eq!(&mapa[..9], [0, 0, 0, 5, 12, 0, 0, 0, 0]);
    }

    #[test]
    fn mapa_curto_ou_tamanho_invalido_nao_chama_o_responder() {
        for mut mapa in [
            vec![],
            vec![0, 0, 0, 0],
            vec![0, 0, 0],
            // Declara 100 bytes e o mapa só guarda 20.
            mapa_com(&[1; 4], 20)
                .into_iter()
                .enumerate()
                .map(|(i, b)| if i == 3 { 100 } else { b })
                .collect(),
            // Declara mais que o máximo do PuTTY.
            {
                let mut mapa = vec![0u8; AGENT_MAX_MSGLEN + 16];
                mapa[..4].copy_from_slice(&(AGENT_MAX_MSGLEN as u32 + 1).to_be_bytes());
                mapa
            },
        ] {
            let copia = mapa.clone();
            let respondeu = copydata_exchange(&mut mapa, |_| panic!("não devia responder"));
            assert!(!respondeu);
            assert_eq!(mapa, copia, "o mapa não muda");
        }
    }

    #[test]
    fn resposta_maior_que_o_mapa_vira_failure() {
        let mut mapa = mapa_com(&[11], 16);
        assert!(copydata_exchange(&mut mapa, |_| vec![0; 100]));
        assert_eq!(&mapa[..5], [0, 0, 0, 1, 5]);
    }

    #[test]
    fn a_resposta_que_cabe_justa_no_mapa_e_gravada() {
        let mut mapa = mapa_com(&[11], 12);
        assert!(copydata_exchange(&mut mapa, |_| vec![7; 8]));
        assert_eq!(&mapa[..4], [0, 0, 0, 8]);
        assert_eq!(&mapa[4..], [7; 8]);
    }
}
