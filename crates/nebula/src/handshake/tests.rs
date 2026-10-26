//! Os testes do aperto de mão: as duas pontas se entendem, as versões e as cifras casam, e
//! cada recusa dá a mensagem certa.

use super::*;
use crate::config::Cipher;
use crate::testutil::{AGORA_NANOS, creds};

fn par(a: &Arc<Credentials>, b: &Arc<Credentials>, versao: Option<Version>) -> (Machine, Machine) {
    (
        Machine::new(a.clone(), true, 0x1111, versao),
        Machine::new(b.clone(), false, 0x2222, None),
    )
}

/// Roda o aperto de mão inteiro entre as duas pontas.
fn completar(ini: &mut Machine, resp: &mut Machine) -> (Completed, Completed) {
    let msg1 = ini.initiate(AGORA_NANOS).unwrap();
    let passo = resp.process(&msg1, AGORA_NANOS + 1).unwrap();
    let resposta = passo.reply.expect("o respondedor responde");
    let do_resp = passo.done.expect("o respondedor termina com a resposta");
    let passo = ini.process(&resposta, AGORA_NANOS + 2).unwrap();
    assert!(passo.reply.is_none());
    (passo.done.expect("o iniciador termina"), do_resp)
}

fn confere_transporte(ini: &Completed, resp: &Completed) {
    let ad = [7u8; 16];
    let cifrado = ini.send.seal(3, &ad, b"do iniciador").unwrap();
    assert_eq!(resp.recv.open(3, &ad, &cifrado).unwrap(), b"do iniciador");
    let cifrado = resp.send.seal(3, &ad, b"do respondedor").unwrap();
    assert_eq!(ini.recv.open(3, &ad, &cifrado).unwrap(), b"do respondedor");
    // Uma cifra não abre o que a própria cifra mandou: as chaves são uma por sentido.
    let cifrado = ini.send.seal(4, &ad, b"x").unwrap();
    assert!(ini.recv.open(4, &ad, &cifrado).is_none());
}

#[test]
fn a_carga_vai_e_volta_e_omite_o_que_e_zero() {
    let carga = Payload {
        cert: vec![1, 2, 3],
        initiator_index: 7,
        responder_index: 0,
        time: 1_700_000_000_000_000_000,
        cert_version: 2,
    };
    let bytes = carga.marshal();
    assert_eq!(Payload::unmarshal(&bytes).unwrap(), carga);
    // Só o necessário: a tag do envelope, o tamanho e os quatro campos presentes.
    let vazia = Payload::default().marshal();
    assert_eq!(vazia, [0x0a, 0x00]);
    assert_eq!(Payload::unmarshal(&vazia).unwrap(), Payload::default());
}

#[test]
fn a_carga_e_rigida_com_os_tipos_e_tolerante_com_o_resto() {
    // O índice do iniciador (campo 2) vindo como bytes: erro.
    assert!(Payload::unmarshal(&[0x0a, 0x03, 0x12, 0x01, 0x00]).is_err());
    // Um índice que não cabe em 32 bits: erro.
    let mut grande = vec![0x0a, 0x07, 0x10];
    grande.extend_from_slice(&[0xff, 0xff, 0xff, 0xff, 0x0f, 0x00]);
    grande[1] = (grande.len() - 2) as u8;
    grande[2] = 0x10;
    assert!(Payload::unmarshal(&grande).is_err());
    // Campo repetido: vale o último. Campo desconhecido: ignorado. Envelope extra: ignorado.
    let bytes = [
        0x0a, 0x0c, 0x10, 0x05, 0x10, 0x09, 0x18, 0x03, 0x48, 0x01, 0x52, 0x02, 0xaa, 0xbb, 0x1a,
        0x01, 0x00,
    ];
    let carga = Payload::unmarshal(&bytes).unwrap();
    assert_eq!(carga.initiator_index, 9);
    assert_eq!(carga.responder_index, 3);
    // Truncado: erro.
    assert!(Payload::unmarshal(&[0x0a, 0x05, 0x10]).is_err());
    assert!(Payload::unmarshal(&[0x0a]).is_err());
}

#[test]
fn as_duas_pontas_se_entendem_com_aes_e_v2() {
    let a = creds("no-v2", "ca-25519", "aes", None);
    let b = creds("no-v2-b", "ca-25519", "aes", None);
    let (mut ini, mut resp) = par(&a, &b, None);
    let (do_ini, do_resp) = completar(&mut ini, &mut resp);
    assert!(do_ini.initiator && !do_resp.initiator);
    assert_eq!(do_ini.local_index, 0x1111);
    assert_eq!(do_ini.remote_index, 0x2222);
    assert_eq!(do_resp.local_index, 0x2222);
    assert_eq!(do_resp.remote_index, 0x1111);
    assert_eq!(do_ini.message_index, 2);
    assert_eq!(do_resp.message_index, 2);
    assert_eq!(do_ini.remote_cert.name(), "no-v2-b");
    assert_eq!(do_resp.remote_cert.name(), "no-v2");
    assert_eq!(do_ini.my_cert.version(), Version::V2);
    // A hora que cada um recebe é a do outro.
    assert_eq!(do_resp.time, AGORA_NANOS);
    assert_eq!(do_ini.time, AGORA_NANOS + 1);
    confere_transporte(&do_ini, &do_resp);
}

#[test]
fn as_duas_pontas_se_entendem_com_chacha() {
    let a = creds("no-chacha", "ca-25519", "chachapoly", None);
    let b = creds("no-v2-b", "ca-25519", "chachapoly", None);
    assert_eq!(a.cipher, Cipher::ChaChaPoly);
    let (mut ini, mut resp) = par(&a, &b, None);
    let (do_ini, do_resp) = completar(&mut ini, &mut resp);
    confere_transporte(&do_ini, &do_resp);
}

#[test]
fn cifras_diferentes_nao_se_entendem() {
    let a = creds("no-v2", "ca-25519", "aes", None);
    let b = creds("no-v2-b", "ca-25519", "chachapoly", None);
    let (mut ini, mut resp) = par(&a, &b, None);
    let msg1 = ini.initiate(AGORA_NANOS).unwrap();
    // A primeira mensagem do IX vai em claro (ainda não há chave), então o respondedor a lê; a
    // resposta é que o iniciador não autentica, porque o nome do protocolo entra no hash.
    let resposta = resp.process(&msg1, AGORA_NANOS).unwrap().reply.unwrap();
    let erro = ini.process(&resposta, AGORA_NANOS).err().expect("recusa");
    assert!(!erro.fatal, "{erro}");
}

#[test]
fn v1_com_v1_e_v1_com_v2() {
    let a = creds("no-v1", "ca-25519", "aes", None);
    let b = creds("no-v1-b", "ca-25519", "aes", None);
    let (mut ini, mut resp) = par(&a, &b, None);
    let (do_ini, do_resp) = completar(&mut ini, &mut resp);
    assert_eq!(do_ini.my_cert.version(), Version::V1);
    assert_eq!(do_resp.my_cert.version(), Version::V1);
    confere_transporte(&do_ini, &do_resp);

    // Quem só tem v1 inicia com um nó só v2: o respondedor confere o v1 (a CA v2 assina v1) e
    // responde com o seu v2, porque não tem v1.
    let c = creds("no-v2", "ca-25519", "aes", None);
    let (mut ini, mut resp) = par(&a, &c, None);
    let (do_ini, do_resp) = completar(&mut ini, &mut resp);
    assert_eq!(do_ini.my_cert.version(), Version::V1);
    assert_eq!(do_resp.my_cert.version(), Version::V2);
    assert_eq!(do_ini.remote_cert.version(), Version::V2);
    assert_eq!(do_resp.remote_cert.version(), Version::V1);
    confere_transporte(&do_ini, &do_resp);
}

#[test]
fn o_respondedor_com_v1_e_v2_acompanha_a_versao_do_iniciador() {
    // O respondedor tem os dois e, por padrão, começa pela v2; o iniciador v1 o puxa para a v1.
    let a = creds("no-v1", "ca-25519", "aes", None);
    let b = creds("no-v1v2-b", "ca-25519", "aes", None);
    assert_eq!(
        b.initiating,
        Version::V1,
        "sem o v2 pedido, o padrão é a v1"
    );
    let (mut ini, mut resp) = par(&a, &b, None);
    let (do_ini, do_resp) = completar(&mut ini, &mut resp);
    assert_eq!(do_resp.my_cert.version(), Version::V1);
    assert_eq!(do_ini.remote_cert.version(), Version::V1);

    // Com `initiating_version: 2` no respondedor, ele ainda segue a versão do iniciador.
    let b = creds("no-v1v2-b", "ca-25519", "aes", Some(2));
    let (mut ini, mut resp) = par(&a, &b, None);
    let (_, do_resp) = completar(&mut ini, &mut resp);
    assert_eq!(do_resp.my_cert.version(), Version::V1);

    // E o iniciador com os dois certificados inicia pela versão pedida, ou pela `version`.
    let c = creds("no-v1v2", "ca-25519", "aes", Some(2));
    let d = creds("no-v2-b", "ca-25519", "aes", None);
    let (mut ini, mut resp) = par(&c, &d, None);
    let (do_ini, _) = completar(&mut ini, &mut resp);
    assert_eq!(do_ini.my_cert.version(), Version::V2);
    let (mut ini, mut resp) = par(&c, &d, Some(Version::V1));
    let (do_ini, _) = completar(&mut ini, &mut resp);
    assert_eq!(do_ini.my_cert.version(), Version::V1);
}

#[test]
fn certificado_de_outra_ca_e_recusado() {
    let a = creds("no-outra-ca", "ca-outra", "aes", None);
    let b = creds("no-v2-b", "ca-25519", "aes", None);
    let (mut ini, mut resp) = par(&a, &b, None);
    let msg1 = ini.initiate(AGORA_NANOS).unwrap();
    let erro = resp.process(&msg1, AGORA_NANOS).err().expect("recusa");
    assert!(erro.fatal);
    assert!(erro.reason.contains("no-outra-ca"), "{erro}");
    assert!(resp.failed());
    // Depois de fatal, a máquina não aceita mais nada.
    assert!(resp.process(&msg1, AGORA_NANOS).is_err());
}

#[test]
fn certificado_vencido_e_recusado() {
    let a = creds("no-vencido", "ca-25519", "aes", None);
    let b = creds("no-v2-b", "ca-25519", "aes", None);
    let (mut ini, mut resp) = par(&a, &b, None);
    let msg1 = ini.initiate(AGORA_NANOS).unwrap();
    let erro = resp.process(&msg1, AGORA_NANOS).err().expect("recusa");
    assert!(erro.fatal);
    assert!(erro.reason.contains("no-vencido"), "{erro}");
}

#[test]
fn o_aperto_de_mao_com_o_proprio_endereco_e_recusado() {
    let a = creds("no-v2", "ca-25519", "aes", None);
    let (mut ini, mut resp) = par(&a, &a, None);
    let msg1 = ini.initiate(AGORA_NANOS).unwrap();
    let erro = resp.process(&msg1, AGORA_NANOS).err().expect("recusa");
    assert!(erro.fatal);
    assert!(erro.reason.contains("próprio nó"), "{erro}");
}

#[test]
fn lixo_nao_estraga_a_maquina_de_quem_inicia() {
    let a = creds("no-v2", "ca-25519", "aes", None);
    let b = creds("no-v2-b", "ca-25519", "aes", None);
    let (mut ini, mut resp) = par(&a, &b, None);
    let msg1 = ini.initiate(AGORA_NANOS).unwrap();
    let passo = resp.process(&msg1, AGORA_NANOS).unwrap();
    let resposta = passo.reply.unwrap();

    // Uma resposta adulterada em qualquer parte do Noise é descartada sem estragar a máquina.
    for posicao in [header::LEN, header::LEN + 40, resposta.len() - 1] {
        let mut ruim = resposta.clone();
        ruim[posicao] ^= 0x55;
        let erro = ini.process(&ruim, AGORA_NANOS).err().expect("recusa");
        assert!(!erro.fatal, "{erro}");
    }
    // O pacote curto e o subtipo trocado também.
    assert!(
        !ini.process(&resposta[..10], AGORA_NANOS)
            .err()
            .unwrap()
            .fatal
    );
    let mut outro = resposta.clone();
    outro[1] = 9;
    assert!(!ini.process(&outro, AGORA_NANOS).err().unwrap().fatal);
    // E a resposta verdadeira ainda completa.
    let passo = ini.process(&resposta, AGORA_NANOS).unwrap();
    assert!(passo.done.is_some());
}

#[test]
fn o_cabecalho_das_mensagens_leva_o_contador_e_o_indice() {
    let a = creds("no-v2", "ca-25519", "aes", None);
    let b = creds("no-v2-b", "ca-25519", "aes", None);
    let (mut ini, mut resp) = par(&a, &b, None);
    let msg1 = ini.initiate(AGORA_NANOS).unwrap();
    let h1 = Header::parse(&msg1).unwrap();
    assert_eq!(
        (h1.kind, h1.subtype, h1.remote_index, h1.counter),
        (MessageType::Handshake, 0, 0, 1)
    );
    let resposta = resp.process(&msg1, AGORA_NANOS).unwrap().reply.unwrap();
    let h2 = Header::parse(&resposta).unwrap();
    assert_eq!(
        (h2.kind, h2.subtype, h2.remote_index, h2.counter),
        (MessageType::Handshake, 0, 0x1111, 2)
    );
}

#[test]
fn quem_inicia_precisa_chamar_initiate_antes() {
    let a = creds("no-v2", "ca-25519", "aes", None);
    let b = creds("no-v2-b", "ca-25519", "aes", None);
    let (mut ini, mut resp) = par(&a, &b, None);
    let (mut outro, _) = par(&a, &b, None);
    let msg1 = outro.initiate(AGORA_NANOS).unwrap();
    let resposta = resp.process(&msg1, AGORA_NANOS).unwrap().reply.unwrap();
    assert!(ini.process(&resposta, AGORA_NANOS).err().unwrap().fatal);
    // Iniciar duas vezes, ou iniciar como respondedor, é erro.
    let (mut ini, mut resp) = par(&a, &b, None);
    ini.initiate(AGORA_NANOS).unwrap();
    assert!(ini.initiate(AGORA_NANOS).is_err());
    assert!(resp.initiate(AGORA_NANOS).is_err());
}

#[test]
fn uma_mensagem_sem_carga_e_recusada() {
    // Um Noise válido cuja carga vai vazia: monta à mão com o `Ix`.
    let a = creds("no-v2", "ca-25519", "aes", None);
    let b = creds("no-v2-b", "ca-25519", "aes", None);
    let mut segredo = [0u8; 32];
    segredo.copy_from_slice(a.key.secret());
    let publica = a.key.public_key().unwrap();
    let mut noise = Ix::new(a.cipher, true, segredo, publica, &[]);
    let mensagem = noise.write_message(&[]).unwrap();
    let mut pacote = header::encode(MessageType::Handshake, 0, 0, 1).to_vec();
    pacote.extend_from_slice(&mensagem);
    let mut resp = Machine::new(b, false, 5, None);
    let erro = resp.process(&pacote, AGORA_NANOS).err().expect("recusa");
    assert!(erro.fatal, "{erro}");
}
