//! Os testes dos formatos (M44): os vetores reais de `vetores/` (feitos com o `ssh-keygen` e o
//! `puttygen`, ver o `gerar.sh`), a ida e volta de cada formato e a conferência contra as
//! ferramentas de verdade, quando elas existem na máquina.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use ssh_key::private::{KeypairData, SkEd25519};
use ssh_key::public::{Ed25519PublicKey, SkEd25519 as SkEd25519Public};
use ssh_key::{HashAlg, PrivateKey, PublicKey};

use super::*;

const SENHA: &str = "senha1";

fn vetor(nome: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/ssh/keys/vetores")
        .join(nome);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn fp_do_vetor(nome: &str) -> String {
    vetor(nome).trim().to_owned()
}

/// Uma pasta temporária por teste, apagada no fim.
struct Pasta(PathBuf);

impl Pasta {
    fn nova(nome: &str) -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let pasta = std::env::temp_dir().join(format!(
            "tomni-ui-formatos-{nome}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).unwrap();
        Self(pasta)
    }

    fn arquivo(&self, nome: &str) -> PathBuf {
        self.0.join(nome)
    }
}

impl Drop for Pasta {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Se o programa existe nesta máquina (os testes contra as ferramentas reais se dão por
/// satisfeitos sem ele, e dizem isso).
fn tem(programa: &str, arg: &str) -> bool {
    let tem = Command::new(programa)
        .arg(arg)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok();
    if !tem {
        eprintln!("{programa} não existe nesta máquina: teste pulado");
    }
    tem
}

fn abrir(texto: &str, senha: Option<&str>) -> KeyFile {
    KeyFile::from_text(texto, senha).unwrap_or_else(|error| panic!("{error}"))
}

fn fp(file: &KeyFile) -> String {
    fingerprint(file.private_key().public_key())
}

/// Todos os vetores de chave privada: arquivo, senha e o arquivo `.fp` esperado.
const PRIVADAS: [(&str, Option<&str>, &str); 15] = [
    ("ed", None, "ed.fp"),
    ("ed-cifrada", Some(SENHA), "ed-cifrada.fp"),
    ("rsa", None, "rsa.fp"),
    ("ec", None, "ec.fp"),
    ("rsa-pem", None, "rsa.fp"),
    ("rsa-pem-cifrada", Some(SENHA), "rsa.fp"),
    ("ec-pem", None, "ec.fp"),
    ("rsa-p8", None, "rsa.fp"),
    ("rsa-p8-cifrada", Some(SENHA), "rsa.fp"),
    ("ec-p8", None, "ec.fp"),
    ("ed-v3.ppk", None, "ed.fp"),
    ("ed-v2.ppk", None, "ed.fp"),
    ("rsa-v3-cifrada.ppk", Some(SENHA), "rsa.fp"),
    ("rsa-v2-cifrada.ppk", Some(SENHA), "rsa.fp"),
    ("ec-v3.ppk", None, "ec.fp"),
];

#[test]
fn cada_vetor_abre_com_o_fingerprint_do_ssh_keygen() {
    for (nome, senha, esperado) in PRIVADAS {
        let texto = vetor(nome);
        let aberta = abrir(&texto, senha);
        assert_eq!(fp(&aberta), fp_do_vetor(esperado), "{nome}");
        if senha.is_some() {
            assert!(matches!(
                KeyFile::from_text(&texto, None),
                Err(KeyError::NeedsPassphrase)
            ));
            assert!(
                matches!(
                    KeyFile::from_text(&texto, Some("errada")),
                    Err(KeyError::WrongPassphrase)
                ),
                "{nome}: senha errada"
            );
        }
    }
}

#[test]
fn a_chave_dos_vetores_assina_e_a_publica_confere() {
    // O fingerprint só prova a metade pública: assinar prova que a privada casa com ela (um
    // erro na ordem dos bytes do PPK passaria batido no fingerprint).
    for (nome, senha, _) in PRIVADAS {
        let aberta = abrir(&vetor(nome), senha);
        let key = aberta.private_key();
        let assinatura = key.sign("teste", HashAlg::Sha256, b"mensagem").unwrap();
        key.public_key()
            .verify("teste", b"mensagem", &assinatura)
            .unwrap_or_else(|error| panic!("{nome}: {error}"));
    }
}

#[test]
fn inspect_text_diz_o_formato_e_a_versao_do_ppk() {
    let formato = |nome: &str| KeyFile::inspect_text(&vetor(nome)).unwrap();
    assert_eq!(formato("ed-v3.ppk").format, KeyFormat::PpkV3);
    assert_eq!(formato("ed-v2.ppk").format, KeyFormat::PpkV2);
    assert!(!formato("ed-v3.ppk").encrypted);
    assert!(formato("rsa-v2-cifrada.ppk").encrypted);
    assert_eq!(formato("ed").format, KeyFormat::OpenSsh);
    assert!(formato("ed-cifrada").encrypted);
    assert_eq!(formato("rsa-pem").format, KeyFormat::Pem);
    assert_eq!(formato("ec-pem").format, KeyFormat::Pem);
    assert_eq!(formato("rsa-p8").format, KeyFormat::Pkcs8);
    assert_eq!(formato("rsa-p8-cifrada").format, KeyFormat::Pkcs8);
    assert!(formato("rsa-p8-cifrada").encrypted);
    assert_eq!(formato("ed").kind, "ED25519");
    assert_eq!(formato("rsa").bits, 2048);
    assert_eq!(formato("ed").comment, "ed");
}

#[test]
fn uma_publica_no_lugar_da_privada_e_so_publica() {
    for nome in ["ed.pub", "ed.rfc4716", "ec.spki.pem", "rsa.pkcs1.pem"] {
        assert!(
            matches!(
                KeyFile::from_text(&vetor(nome), None),
                Err(KeyError::OnlyPublic)
            ),
            "{nome}"
        );
    }
    assert!(matches!(
        KeyFile::from_text("isto não é uma chave", None),
        Err(KeyError::Invalid(_))
    ));
}

#[test]
fn ida_e_volta_em_todos_os_formatos() {
    for (nome, comentario) in [("ed", "ed"), ("rsa", "rsa"), ("ec", "ec")] {
        let original = abrir(&vetor(nome), None);
        let key = original.private_key();
        for formato in PrivateFormat::available(key) {
            let senhas: &[Option<&str>] = if formato.encrypts() {
                &[None, Some("uma frase longa")]
            } else {
                &[None]
            };
            for &senha in senhas {
                let texto = export_private(key, formato, senha, KdfLevel::Standard)
                    .unwrap_or_else(|error| panic!("{nome} {formato:?}: {error}"));
                let volta = abrir(&texto, senha);
                assert_eq!(fp(&volta), fp(&original), "{nome} {formato:?} {senha:?}");
                let assinatura = volta
                    .private_key()
                    .sign("x", HashAlg::Sha256, b"m")
                    .unwrap();
                original
                    .private_key()
                    .public_key()
                    .verify("x", b"m", &assinatura)
                    .unwrap();
                // O PKCS#8 e o PEM tradicional não têm onde guardar o comentário.
                let guarda = matches!(
                    formato,
                    PrivateFormat::OpenSsh | PrivateFormat::PpkV2 | PrivateFormat::PpkV3
                );
                if guarda {
                    assert_eq!(volta.private_key().comment().to_string(), comentario);
                }
                assert_eq!(
                    KeyFile::inspect_text(&texto).unwrap().encrypted,
                    senha.is_some(),
                    "{nome} {formato:?}"
                );
            }
        }
    }
}

#[test]
fn os_cabecalhos_de_cada_formato_sao_os_esperados() {
    let ed = abrir(&vetor("ed"), None);
    let rsa = abrir(&vetor("rsa"), None);
    let ec = abrir(&vetor("ec"), None);
    let texto = |chave: &KeyFile, formato, senha| {
        export_private(chave.private_key(), formato, senha, KdfLevel::Standard).unwrap()
    };
    assert!(
        texto(&ed, PrivateFormat::OpenSsh, None)
            .starts_with("-----BEGIN OPENSSH PRIVATE KEY-----\n")
    );
    assert!(texto(&ed, PrivateFormat::Pkcs8, None).starts_with("-----BEGIN PRIVATE KEY-----\n"));
    assert!(
        texto(&ed, PrivateFormat::Pkcs8, Some(SENHA))
            .starts_with("-----BEGIN ENCRYPTED PRIVATE KEY-----\n")
    );
    assert!(
        texto(&rsa, PrivateFormat::PemLegacy, None)
            .starts_with("-----BEGIN RSA PRIVATE KEY-----\n")
    );
    assert!(
        texto(&ec, PrivateFormat::PemLegacy, None).starts_with("-----BEGIN EC PRIVATE KEY-----\n")
    );
    assert!(
        texto(&ed, PrivateFormat::PpkV3, None).starts_with("PuTTY-User-Key-File-3: ssh-ed25519\n")
    );
    assert!(
        texto(&rsa, PrivateFormat::PpkV2, None).starts_with("PuTTY-User-Key-File-2: ssh-rsa\n")
    );
    let cifrado = texto(&ed, PrivateFormat::PpkV3, Some(SENHA));
    for linha in [
        "Encryption: aes256-cbc",
        "Key-Derivation: Argon2id",
        "Argon2-Memory: 8192",
        "Argon2-Passes: 13",
        "Argon2-Parallelism: 1",
    ] {
        assert!(cifrado.contains(linha), "falta {linha}");
    }
    // O PEM de verdade tem 64 colunas.
    for formato in [PrivateFormat::Pkcs8, PrivateFormat::PemLegacy] {
        for linha in texto(&rsa, formato, None).lines() {
            assert!(linha.len() <= 64, "{formato:?}: {linha}");
        }
    }
}

#[test]
fn pem_legado_de_ed25519_e_recusado() {
    let ed = abrir(&vetor("ed"), None);
    assert!(!PrivateFormat::available(ed.private_key()).contains(&PrivateFormat::PemLegacy));
    let erro = export_private(
        ed.private_key(),
        PrivateFormat::PemLegacy,
        None,
        KdfLevel::Standard,
    )
    .unwrap_err();
    assert!(
        matches!(erro, KeyError::FormatNotSupported { .. }),
        "{erro}"
    );
    assert!(erro.to_string().contains("ED25519"));
}

#[test]
fn pem_legado_com_passphrase_e_recusado() {
    let rsa = abrir(&vetor("rsa"), None);
    let erro = export_private(
        rsa.private_key(),
        PrivateFormat::PemLegacy,
        Some(SENHA),
        KdfLevel::Standard,
    )
    .unwrap_err();
    assert!(matches!(erro, KeyError::PassphraseNotSupported), "{erro}");
    assert!(!PrivateFormat::PemLegacy.encrypts());
}

#[test]
fn ppk_gravado_abre_no_ssh_key() {
    // O `ssh-key` lê o que o `ppk.rs` grava, nas duas versões, com e sem passphrase.
    for nome in ["ed", "rsa", "ec"] {
        let original = abrir(&vetor(nome), None);
        for (formato, versao) in [(PrivateFormat::PpkV3, 3), (PrivateFormat::PpkV2, 2)] {
            for senha in [None, Some("frase")] {
                let texto =
                    export_private(original.private_key(), formato, senha, KdfLevel::Standard)
                        .unwrap();
                assert!(texto.contains(&format!("PuTTY-User-Key-File-{versao}")));
                let lida = PrivateKey::from_ppk(&texto, senha.map(str::to_owned))
                    .unwrap_or_else(|error| panic!("{nome} v{versao} {senha:?}: {error}"));
                assert_eq!(
                    fingerprint(lida.public_key()),
                    fp(&original),
                    "{nome} v{versao}"
                );
                let assinatura = lida.sign("x", HashAlg::Sha256, b"m").unwrap();
                original
                    .private_key()
                    .public_key()
                    .verify("x", b"m", &assinatura)
                    .unwrap_or_else(|error| panic!("{nome} v{versao}: {error}"));
                if senha.is_some() {
                    assert!(
                        PrivateKey::from_ppk(&texto, Some("errada".to_owned())).is_err(),
                        "{nome} v{versao}: a senha errada não pode abrir"
                    );
                }
            }
        }
    }
}

#[test]
fn ed25519_com_o_primeiro_bit_da_semente_ligado_vai_e_volta_no_ppk() {
    // O `puttygen` grava a semente como `string` de 32 bytes mesmo com o bit alto ligado.
    let mut visto = 0;
    for _ in 0..40 {
        let gerada = KeyGen::new(KeyType::Ed25519)
            .comment("x")
            .generate()
            .unwrap();
        let KeypairData::Ed25519(par) = gerada.private_key().key_data() else {
            unreachable!();
        };
        if par.private.to_bytes()[0] & 0x80 == 0 {
            continue;
        }
        visto += 1;
        for formato in [PrivateFormat::PpkV3, PrivateFormat::PpkV2] {
            let texto =
                export_private(gerada.private_key(), formato, Some("a"), KdfLevel::Standard)
                    .unwrap();
            let volta = abrir(&texto, Some("a"));
            let assinatura = volta
                .private_key()
                .sign("x", HashAlg::Sha256, b"m")
                .unwrap();
            gerada.public_key().verify("x", b"m", &assinatura).unwrap();
        }
    }
    assert!(
        visto > 0,
        "40 chaves sem nenhuma de bit alto é improvável demais"
    );
}

#[test]
fn ppk_gravado_abre_no_puttygen() {
    if !tem("puttygen", "--version") {
        return;
    }
    let pasta = Pasta::nova("puttygen");
    for nome in ["ed", "rsa", "ec"] {
        let original = abrir(&vetor(nome), None);
        for formato in [PrivateFormat::PpkV3, PrivateFormat::PpkV2] {
            for senha in [None, Some(SENHA)] {
                let texto =
                    export_private(original.private_key(), formato, senha, KdfLevel::Standard)
                        .unwrap();
                let saida = pasta.arquivo("saida.ppk");
                write_key_text(&saida, &texto, true, true).unwrap();
                let mut comando = Command::new("puttygen");
                comando.arg(&saida).args(["-O", "fingerprint"]);
                if senha.is_some() {
                    let arquivo = pasta.arquivo("senha.txt");
                    fs::write(&arquivo, format!("{SENHA}\n")).unwrap();
                    comando.arg("--old-passphrase").arg(&arquivo);
                }
                let resposta = comando.output().unwrap();
                let saida = String::from_utf8_lossy(&resposta.stdout);
                assert!(
                    resposta.status.success() && saida.contains(&fp(&original)),
                    "{nome} {formato:?} {senha:?}: {saida} {}",
                    String::from_utf8_lossy(&resposta.stderr)
                );
            }
        }
    }
}

#[test]
fn pem_e_pkcs8_gravados_abrem_no_ssh_keygen() {
    if !tem("ssh-keygen", "-?") {
        return;
    }
    let pasta = Pasta::nova("keygen");
    for nome in ["ed", "rsa", "ec"] {
        let original = abrir(&vetor(nome), None);
        let esperada = {
            let linha = public_openssh(original.private_key().public_key());
            linha
                .split_whitespace()
                .take(2)
                .collect::<Vec<_>>()
                .join(" ")
        };
        for formato in PrivateFormat::available(original.private_key()) {
            if formato == PrivateFormat::PpkV3 || formato == PrivateFormat::PpkV2 {
                continue;
            }
            // O `ssh-keygen` não lê Ed25519 em PKCS#8 (quem lê é o OpenSSL, testado à parte).
            if nome == "ed" && formato == PrivateFormat::Pkcs8 {
                continue;
            }
            let senhas: &[Option<&str>] = if formato.encrypts() {
                &[None, Some(SENHA)]
            } else {
                &[None]
            };
            for &senha in senhas {
                let texto =
                    export_private(original.private_key(), formato, senha, KdfLevel::Standard)
                        .unwrap();
                let saida = pasta.arquivo("saida");
                write_key_text(&saida, &texto, true, true).unwrap();
                let resposta = Command::new("ssh-keygen")
                    .args(["-y", "-P", senha.unwrap_or(""), "-f"])
                    .arg(&saida)
                    .output()
                    .unwrap();
                let linha = String::from_utf8_lossy(&resposta.stdout);
                let obtida = linha
                    .split_whitespace()
                    .take(2)
                    .collect::<Vec<_>>()
                    .join(" ");
                assert!(
                    resposta.status.success() && obtida == esperada,
                    "{nome} {formato:?} {senha:?}: {linha} {}",
                    String::from_utf8_lossy(&resposta.stderr)
                );
            }
        }
    }
}

#[test]
fn publicas_gravadas_abrem_no_ssh_keygen() {
    if !tem("ssh-keygen", "-?") {
        return;
    }
    let pasta = Pasta::nova("publicas");
    for nome in ["ed", "rsa", "ec"] {
        let (publica, _) = parse_public(&vetor(&format!("{nome}.pub"))).unwrap();
        let esperada = public_openssh(&publica)
            .split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ");
        for (formato, metodo) in [
            (PublicFormat::Rfc4716, "RFC4716"),
            (PublicFormat::SpkiPem, "PKCS8"),
            (PublicFormat::Pkcs1Pem, "PEM"),
        ] {
            if !PublicFormat::available(&publica).contains(&formato) {
                continue;
            }
            // O `ssh-keygen -i -m PKCS8` não conhece Ed25519 (o OpenSSL conhece, testado à parte).
            if nome == "ed" && formato == PublicFormat::SpkiPem {
                continue;
            }
            let texto = export_public(&publica, "um comentário com espaços", formato).unwrap();
            let saida = pasta.arquivo("saida.pub");
            fs::write(&saida, &texto).unwrap();
            let resposta = Command::new("ssh-keygen")
                .args(["-i", "-m", metodo, "-f"])
                .arg(&saida)
                .output()
                .unwrap();
            let linha = String::from_utf8_lossy(&resposta.stdout);
            let obtida = linha
                .split_whitespace()
                .take(2)
                .collect::<Vec<_>>()
                .join(" ");
            assert!(
                resposta.status.success() && obtida == esperada,
                "{nome} {formato:?}: {linha} {}",
                String::from_utf8_lossy(&resposta.stderr)
            );
        }
    }
}

#[test]
fn spki_do_ed25519_confere_com_o_openssl() {
    if !tem("openssl", "version") {
        return;
    }
    let pasta = Pasta::nova("openssl");
    // Uma chave de verdade do OpenSSL entra pelo PKCS#8, e a pública sai em SPKI igual à nossa.
    let privada = pasta.arquivo("ed.pem");
    let gerou = Command::new("openssl")
        .args(["genpkey", "-algorithm", "ed25519", "-out"])
        .arg(&privada)
        .status()
        .unwrap();
    assert!(gerou.success());
    let aberta = abrir(&fs::read_to_string(&privada).unwrap(), None);
    assert_eq!(
        KeyFile::inspect_text(&fs::read_to_string(&privada).unwrap())
            .unwrap()
            .format,
        KeyFormat::Pkcs8
    );
    let nossa =
        export_public(aberta.private_key().public_key(), "", PublicFormat::SpkiPem).unwrap();
    let deles = Command::new("openssl")
        .args(["pkey", "-pubout", "-in"])
        .arg(&privada)
        .output()
        .unwrap();
    assert_eq!(nossa, String::from_utf8_lossy(&deles.stdout));
    // E o PKCS#8 que gravamos o OpenSSL lê.
    let nosso = export_private(
        aberta.private_key(),
        PrivateFormat::Pkcs8,
        None,
        KdfLevel::Standard,
    )
    .unwrap();
    let saida = pasta.arquivo("nosso.pem");
    fs::write(&saida, nosso.as_bytes()).unwrap();
    let lido = Command::new("openssl")
        .args(["pkey", "-pubout", "-in"])
        .arg(&saida)
        .output()
        .unwrap();
    assert_eq!(nossa, String::from_utf8_lossy(&lido.stdout));
    // E o cifrado também, com a senha certa.
    let cifrado = export_private(
        aberta.private_key(),
        PrivateFormat::Pkcs8,
        Some(SENHA),
        KdfLevel::Standard,
    )
    .unwrap();
    let saida = pasta.arquivo("cifrado.pem");
    fs::write(&saida, cifrado.as_bytes()).unwrap();
    let lido = Command::new("openssl")
        .args([
            "pkey",
            "-pubout",
            "-passin",
            &format!("pass:{SENHA}"),
            "-in",
        ])
        .arg(&saida)
        .output()
        .unwrap();
    assert_eq!(nossa, String::from_utf8_lossy(&lido.stdout));
    let errada = Command::new("openssl")
        .args(["pkey", "-pubout", "-passin", "pass:errada", "-in"])
        .arg(&saida)
        .output()
        .unwrap();
    assert!(!errada.status.success());
}

#[test]
fn parse_public_le_os_quatro_formatos_e_opcoes_do_authorized_keys() {
    let linha = |nome: &str| vetor(nome).trim().to_owned();
    // OpenSSH, com o comentário.
    let (chave, comentario) = parse_public(&vetor("ed.pub")).unwrap();
    assert_eq!(fingerprint(&chave), fp_do_vetor("ed.fp"));
    assert_eq!(comentario, "ed");
    // RFC 4716 (o comentário do `ssh-keygen -e` é uma frase).
    let (chave, comentario) = parse_public(&vetor("ed.rfc4716")).unwrap();
    assert_eq!(fingerprint(&chave), fp_do_vetor("ed.fp"));
    assert!(!comentario.is_empty());
    // SPKI das três famílias e PKCS#1.
    assert_eq!(
        fingerprint(&parse_public(&vetor("ec.spki.pem")).unwrap().0),
        fp_do_vetor("ec.fp")
    );
    assert_eq!(
        fingerprint(&parse_public(&vetor("rsa.spki.pem")).unwrap().0),
        fp_do_vetor("rsa.fp")
    );
    assert_eq!(
        fingerprint(&parse_public(&vetor("rsa.pkcs1.pem")).unwrap().0),
        fp_do_vetor("rsa.fp")
    );
    // Opções do `authorized_keys`, com espaços dentro das aspas.
    let com_opcoes = format!(
        "# uma chave\nfrom=\"10.0.0.1,*.exemplo.com\",command=\"ssh -v host\",no-pty {}",
        linha("ed.pub")
    );
    let (chave, comentario) = parse_public(&com_opcoes).unwrap();
    assert_eq!(fingerprint(&chave), fp_do_vetor("ed.fp"));
    assert_eq!(comentario, "ed");
    // RFC 4716 com `Subject:` e uma linha de cabeçalho continuada com `\`.
    let publica = linha("ed.pub");
    let blob = publica.split_whitespace().nth(1).unwrap();
    let texto = format!(
        "---- BEGIN SSH2 PUBLIC KEY ----\nSubject: ana\nComment: \"uma frase \\\ncontinuada\"\n{blob}\n---- END SSH2 PUBLIC KEY ----\n"
    );
    let (chave, comentario) = parse_public(&texto).unwrap();
    assert_eq!(fingerprint(&chave), fp_do_vetor("ed.fp"));
    assert_eq!(comentario, "uma frase continuada");
    // Lixo.
    assert!(parse_public("nada disto é uma chave").is_err());
    assert!(parse_public("-----BEGIN PUBLIC KEY-----\nAAAA\n-----END PUBLIC KEY-----").is_err());
}

#[test]
fn as_publicas_exportadas_batem_com_as_do_ssh_keygen_e_voltam() {
    for (nome, spki, pkcs1) in [
        ("ed", false, false),
        ("ec", true, false),
        ("rsa", true, true),
    ] {
        let (publica, _) = parse_public(&vetor(&format!("{nome}.pub"))).unwrap();
        for formato in PublicFormat::available(&publica) {
            let texto = export_public(&publica, "volta", formato).unwrap();
            let (lida, comentario) = parse_public(&texto).unwrap();
            assert_eq!(
                fingerprint(&lida),
                fingerprint(&publica),
                "{nome} {formato:?}"
            );
            if matches!(formato, PublicFormat::OpenSsh | PublicFormat::Rfc4716) {
                assert_eq!(comentario, "volta", "{nome} {formato:?}");
            }
        }
        // Os dois PEM saem byte a byte como os do `ssh-keygen -e`.
        if spki {
            assert_eq!(
                export_public(&publica, "", PublicFormat::SpkiPem).unwrap(),
                vetor(&format!("{nome}.spki.pem")),
                "{nome} SPKI"
            );
        }
        if pkcs1 {
            assert_eq!(
                export_public(&publica, "", PublicFormat::Pkcs1Pem).unwrap(),
                vetor(&format!("{nome}.pkcs1.pem")),
                "{nome} PKCS#1"
            );
        }
    }
    let (ed, _) = parse_public(&vetor("ed.pub")).unwrap();
    assert!(matches!(
        export_public(&ed, "", PublicFormat::Pkcs1Pem),
        Err(KeyError::FormatNotSupported { .. })
    ));
}

#[test]
fn rfc4716_longo_continua_com_barra_e_volta_inteiro() {
    let (publica, _) = parse_public(&vetor("ed.pub")).unwrap();
    let comentario = "um comentário bem comprido ".repeat(6).trim().to_owned();
    let texto = export_public(&publica, &comentario, PublicFormat::Rfc4716).unwrap();
    for linha in texto.lines() {
        assert!(linha.chars().count() <= 72, "{linha}");
    }
    assert_eq!(parse_public(&texto).unwrap().1, comentario);
}

#[test]
fn sk_so_aceita_openssh() {
    // Uma chave de token (FIDO) gerada à mão: a pública é a de qualquer Ed25519 e o "key
    // handle" é um valor fixo. Não precisa de token para listar e gravar.
    let pub_sk = SkEd25519Public::new(Ed25519PublicKey([7; 32]), "ssh:");
    let par = SkEd25519::new(pub_sk, 0x01, vec![9; 64]).unwrap();
    let chave = PrivateKey::new(KeypairData::SkEd25519(par), "token").unwrap();
    assert_eq!(
        PrivateFormat::available(&chave),
        vec![PrivateFormat::OpenSsh]
    );
    let texto = export_private(&chave, PrivateFormat::OpenSsh, None, KdfLevel::Standard).unwrap();
    let volta = PrivateKey::from_openssh(&texto).unwrap();
    assert_eq!(
        fingerprint(volta.public_key()),
        fingerprint(chave.public_key())
    );
    for formato in [
        PrivateFormat::Pkcs8,
        PrivateFormat::PpkV3,
        PrivateFormat::PemLegacy,
    ] {
        assert!(matches!(
            export_private(&chave, formato, None, KdfLevel::Standard),
            Err(KeyError::FormatNotSupported { .. })
        ));
    }
    let publica: &PublicKey = chave.public_key();
    assert_eq!(
        PublicFormat::available(publica),
        vec![PublicFormat::OpenSsh, PublicFormat::Rfc4716]
    );
}

#[test]
fn write_key_text_grava_atomico_com_a_permissao_certa() {
    let pasta = Pasta::nova("escrita");
    let alvo = pasta.arquivo("a/b/chave");
    write_key_text(&alvo, "um", true, false).unwrap();
    assert_eq!(fs::read_to_string(&alvo).unwrap(), "um");
    assert!(!pasta.arquivo("a/b/chave.tmp").exists(), "o .tmp não fica");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let modo = fs::metadata(&alvo).unwrap().permissions().mode() & 0o777;
        assert_eq!(modo, 0o600);
    }
    // Sem `overwrite`, recusa e deixa o arquivo como está.
    assert!(matches!(
        write_key_text(&alvo, "dois", true, false),
        Err(KeyError::Exists(_))
    ));
    assert_eq!(fs::read_to_string(&alvo).unwrap(), "um");
    // Com `overwrite`, troca.
    write_key_text(&alvo, "dois", true, true).unwrap();
    assert_eq!(fs::read_to_string(&alvo).unwrap(), "dois");
    // A pública não é secreta.
    let publica = pasta.arquivo("chave.pub");
    write_key_text(&publica, "x", false, false).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&publica).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }
}
