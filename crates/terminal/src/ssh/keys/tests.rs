//! Os testes das chaves: geração, formato, passphrase, `rewrite` e a interoperabilidade com o
//! `ssh-keygen` do sistema, quando ele existe.

use super::*;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Uma pasta temporária por teste, apagada no fim.
struct Pasta(PathBuf);

impl Pasta {
    fn nova(nome: &str) -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let pasta = std::env::temp_dir().join(format!(
            "tomni-ui-chaves-{nome}-{}-{}",
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

/// As chaves de teste feitas com o `puttygen` (ver o `LEIA.md` da pasta).
fn fixture(nome: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../test/tests/chaves")
        .join(nome)
}

/// Roda um programa do sistema; `None` se ele não existe (o teste é pulado).
fn rodar(programa: &str, args: &[&str]) -> Option<(bool, String)> {
    match Command::new(programa).args(args).output() {
        Ok(saida) => Some((
            saida.status.success(),
            String::from_utf8_lossy(&saida.stdout).into_owned(),
        )),
        Err(_) => {
            eprintln!("{programa} não existe neste sistema: teste pulado");
            None
        }
    }
}

#[test]
fn gera_cada_tipo_e_a_chave_relida_tem_a_mesma_impressao_digital() {
    let pasta = Pasta::nova("tipos");
    let tipos = [
        KeyType::Ed25519,
        KeyType::Ecdsa(EcdsaCurve::NistP256),
        KeyType::Ecdsa(EcdsaCurve::NistP384),
        KeyType::Ecdsa(EcdsaCurve::NistP521),
        KeyType::Rsa(RsaBits::B3072),
    ];
    for (n, tipo) in tipos.into_iter().enumerate() {
        let chave = KeyGen::new(tipo).comment("ana@teste").generate().unwrap();
        let caminho = pasta.arquivo(&format!("chave{n}"));
        chave.write(&caminho, false).unwrap();
        let aberta = KeyFile::open(&caminho, None).unwrap();
        assert_eq!(
            fingerprint(aberta.private_key().public_key()),
            chave.fingerprint(),
            "{}",
            tipo.label()
        );
        assert!(chave.fingerprint().starts_with("SHA256:"));
        assert_eq!(aberta.info().comment, "ana@teste");
        assert!(!aberta.info().encrypted);
        let (nome, bits) = kind_and_bits(chave.public_key());
        assert_eq!(aberta.info().bits, bits);
        assert!(!nome.is_empty());
    }
}

#[test]
#[ignore = "lento: gera RSA 4096"]
fn gera_rsa_4096() {
    let chave = KeyGen::new(KeyType::Rsa(RsaBits::B4096))
        .generate()
        .unwrap();
    assert_eq!(kind_and_bits(chave.public_key()), ("RSA".to_owned(), 4096));
}

#[test]
fn o_tipo_nao_oferece_dsa_nem_rsa_pequeno() {
    assert!(KeyType::parse("dsa", None).is_err());
    assert!(KeyType::parse("rsa", Some(1024)).is_err());
    assert!(KeyType::parse("rsa", Some(2048)).is_err());
    assert!(KeyType::parse("ecdsa", Some(100)).is_err());
    assert!(KeyType::parse("ed25519", Some(512)).is_err());
    assert_eq!(KeyType::parse("ED25519", None), Ok(KeyType::Ed25519));
    assert_eq!(
        KeyType::parse("rsa", None),
        Ok(KeyType::Rsa(RsaBits::B3072))
    );
    assert!(KeyType::ALL.iter().all(|tipo| match tipo {
        KeyType::Rsa(bits) => bits.bits() >= 3072,
        _ => true,
    }));
}

#[test]
fn com_passphrase_a_chave_pede_a_frase_e_recusa_a_errada() {
    let pasta = Pasta::nova("frase");
    let caminho = pasta.arquivo("id");
    let chave = KeyGen::new(KeyType::Ed25519)
        .passphrase("uma frase")
        .generate()
        .unwrap();
    chave.write(&caminho, false).unwrap();
    assert!(KeyFile::inspect(&caminho).unwrap().encrypted);
    assert!(matches!(
        KeyFile::open(&caminho, None),
        Err(KeyError::NeedsPassphrase)
    ));
    assert!(matches!(
        KeyFile::open(&caminho, Some("outra")),
        Err(KeyError::WrongPassphrase)
    ));
    let aberta = KeyFile::open(&caminho, Some("uma frase")).unwrap();
    assert_eq!(
        fingerprint(aberta.private_key().public_key()),
        chave.fingerprint()
    );
    assert_eq!(
        KeyError::WrongPassphrase.to_string(),
        "A passphrase não confere."
    );
}

fn rodadas(caminho: &Path) -> u32 {
    let chave = PrivateKey::from_openssh(fs::read_to_string(caminho).unwrap()).unwrap();
    assert_eq!(chave.cipher(), Cipher::Aes256Ctr);
    match chave.kdf() {
        Kdf::Bcrypt { salt, rounds } => {
            assert_eq!(salt.len(), 16);
            *rounds
        }
        _ => panic!("sem KDF bcrypt"),
    }
}

#[test]
fn as_rodadas_gravadas_sao_as_do_nivel() {
    let pasta = Pasta::nova("rodadas");
    for (nivel, esperado) in [
        (KdfLevel::Standard, 16),
        (KdfLevel::High, 64),
        (KdfLevel::VeryHigh, 100),
        (KdfLevel::Custom(7), 7),
    ] {
        let caminho = pasta.arquivo(&format!("id{esperado}"));
        KeyGen::new(KeyType::Ed25519)
            .passphrase("p")
            .kdf(nivel)
            .generate()
            .unwrap()
            .write(&caminho, false)
            .unwrap();
        assert_eq!(rodadas(&caminho), esperado);
    }
}

#[test]
fn nao_sobrescreve_sem_pedir_e_grava_com_as_permissoes_do_openssh() {
    let pasta = Pasta::nova("grava");
    let caminho = pasta.arquivo("novo/id");
    let primeira = KeyGen::new(KeyType::Ed25519).generate().unwrap();
    primeira.write(&caminho, false).unwrap();
    let segunda = KeyGen::new(KeyType::Ed25519).generate().unwrap();
    assert!(matches!(
        segunda.write(&caminho, false),
        Err(KeyError::Exists(_))
    ));
    assert_eq!(
        fs::read_to_string(&caminho).unwrap(),
        primeira.private_openssh().as_str()
    );
    segunda.write(&caminho, true).unwrap();
    assert_eq!(
        KeyFile::open(&caminho, None)
            .unwrap()
            .info()
            .public
            .as_ref()
            .map(fingerprint),
        Some(segunda.fingerprint())
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let modo = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(modo(&caminho), 0o600);
        assert_eq!(modo(&public_path(&caminho)), 0o644);
        assert_eq!(modo(caminho.parent().unwrap()), 0o700);
    }
}

#[test]
fn rewrite_poe_troca_e_tira_a_passphrase() {
    let pasta = Pasta::nova("rewrite");
    let caminho = pasta.arquivo("id");
    let chave = KeyGen::new(KeyType::Ed25519)
        .comment("ana@rewrite")
        .generate()
        .unwrap();
    chave.write(&caminho, false).unwrap();
    let impressao = chave.fingerprint();

    let aberta = KeyFile::open(&caminho, None).unwrap();
    aberta
        .rewrite(&caminho, Some("primeira"), KdfLevel::High)
        .unwrap();
    assert_eq!(rodadas(&caminho), 64);
    let aberta = KeyFile::open(&caminho, Some("primeira")).unwrap();
    assert_eq!(fingerprint(aberta.private_key().public_key()), impressao);
    assert_eq!(aberta.info().comment, "ana@rewrite");

    aberta
        .rewrite(&caminho, Some("segunda"), KdfLevel::Standard)
        .unwrap();
    assert!(matches!(
        KeyFile::open(&caminho, Some("primeira")),
        Err(KeyError::WrongPassphrase)
    ));
    let aberta = KeyFile::open(&caminho, Some("segunda")).unwrap();

    aberta.rewrite(&caminho, None, KdfLevel::Standard).unwrap();
    let aberta = KeyFile::open(&caminho, None).unwrap();
    assert!(!aberta.info().encrypted);
    assert_eq!(fingerprint(aberta.private_key().public_key()), impressao);
    // O `.pub` acompanha.
    let linha = fs::read_to_string(public_path(&caminho)).unwrap();
    assert_eq!(linha.trim(), chave.public_openssh());
}

#[test]
fn um_ppk_vira_openssh_e_o_original_fica() {
    let pasta = Pasta::nova("ppk");
    let origem = pasta.arquivo("putty.ppk");
    fs::copy(fixture("ed25519.ppk"), &origem).unwrap();

    let info = KeyFile::inspect(&origem).unwrap();
    assert_eq!(info.format, KeyFormat::PpkV3);
    assert!(!info.encrypted);
    assert_eq!((info.kind.as_str(), info.bits), ("ED25519", 256));
    assert_eq!(info.comment, "ana@putty");
    assert_eq!(
        info.fingerprint().as_deref(),
        Some("SHA256:Asnk9ujhlHlyatX4tVkw+ddroiIzjD3DrfEyC/JK89M")
    );

    let aberta = KeyFile::open(&origem, None).unwrap();
    let destino = aberta.rewrite(&origem, None, KdfLevel::Standard).unwrap();
    assert_eq!(destino, pasta.arquivo("putty"));
    assert!(origem.exists(), "o .ppk original fica");
    let nova = KeyFile::open(&destino, None).unwrap();
    assert_eq!(nova.info().format, KeyFormat::OpenSsh);
    assert_eq!(
        fingerprint(nova.private_key().public_key()),
        "SHA256:Asnk9ujhlHlyatX4tVkw+ddroiIzjD3DrfEyC/JK89M"
    );
    // Converter de novo não sobrescreve o que já existe.
    assert!(matches!(
        aberta.rewrite(&origem, None, KdfLevel::Standard),
        Err(KeyError::Exists(_))
    ));
}

#[test]
fn um_ppk_rsa_com_passphrase_abre_so_com_a_frase() {
    let origem = fixture("rsa-senha.ppk");
    let info = KeyFile::inspect(&origem).unwrap();
    assert!(info.encrypted);
    assert_eq!((info.kind.as_str(), info.bits), ("RSA", 3072));
    assert!(matches!(
        KeyFile::open(&origem, None),
        Err(KeyError::NeedsPassphrase)
    ));
    assert!(matches!(
        KeyFile::open(&origem, Some("errada")),
        Err(KeyError::WrongPassphrase)
    ));
    let aberta = KeyFile::open(&origem, Some("putty-senha")).unwrap();
    assert_eq!(
        Some(fingerprint(aberta.private_key().public_key())),
        info.fingerprint()
    );
}

#[test]
fn le_pem_pkcs1_e_pkcs8_do_openssl() {
    let pasta = Pasta::nova("pem");
    let origem = pasta.arquivo("id");
    let Some(_) = rodar(
        "ssh-keygen",
        &[
            "-q",
            "-t",
            "rsa",
            "-b",
            "3072",
            "-m",
            "PEM",
            "-N",
            "",
            "-f",
            origem.to_str().unwrap(),
        ],
    ) else {
        return;
    };
    let info = KeyFile::inspect(&origem).unwrap();
    assert_eq!(info.format, KeyFormat::Pem);
    assert_eq!((info.kind.as_str(), info.bits), ("RSA", 3072));
    let aberta = KeyFile::open(&origem, None).unwrap();
    // O `.pub` do `ssh-keygen` e a chave lida do PEM batem.
    let (_, publica) = rodar("ssh-keygen", &["-y", "-f", origem.to_str().unwrap()]).unwrap();
    assert_eq!(
        public_openssh(aberta.private_key().public_key())
            .split(' ')
            .nth(1),
        publica.trim().split(' ').nth(1)
    );

    let pkcs8 = pasta.arquivo("id8");
    fs::copy(&origem, &pkcs8).unwrap();
    // Converte para PKCS#8 com a passphrase "p8-senha".
    let Some((ok, _)) = rodar(
        "ssh-keygen",
        &[
            "-p",
            "-m",
            "PKCS8",
            "-P",
            "",
            "-N",
            "p8-senha",
            "-f",
            pkcs8.to_str().unwrap(),
        ],
    ) else {
        return;
    };
    assert!(ok);
    let info = KeyFile::inspect(&pkcs8).unwrap();
    assert_eq!(info.format, KeyFormat::Pkcs8);
    assert!(info.encrypted);
    assert!(matches!(
        KeyFile::open(&pkcs8, Some("outra")),
        Err(KeyError::WrongPassphrase)
    ));
    let aberta = KeyFile::open(&pkcs8, Some("p8-senha")).unwrap();
    assert_eq!(
        public_openssh(aberta.private_key().public_key())
            .split(' ')
            .nth(1),
        publica.trim().split(' ').nth(1)
    );
}

/// O quadro e a impressão digital de uma chave fixa, para o caso sem o `ssh-keygen`.
#[test]
fn o_randomart_de_uma_chave_fixa() {
    let publica = PublicKey::from_openssh(
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIKsvCnHDyrs+BQQsvc4jW438drUqlg3JtudrTHtv1JHj ana@putty",
    )
    .unwrap();
    assert_eq!(
        fingerprint(&publica),
        "SHA256:Asnk9ujhlHlyatX4tVkw+ddroiIzjD3DrfEyC/JK89M"
    );
    let quadro = randomart(&publica);
    let linhas: Vec<&str> = quadro.lines().collect();
    assert_eq!(linhas.len(), 11);
    assert_eq!(linhas[0], "+--[ED25519 256]--+");
    assert_eq!(linhas[10], "+----[SHA256]-----+");
    assert!(linhas.iter().all(|linha| linha.chars().count() == 19));
}

#[test]
fn o_randomart_e_igual_ao_do_ssh_keygen() {
    let pasta = Pasta::nova("art");
    for tipo in [
        KeyType::Ed25519,
        KeyType::Ecdsa(EcdsaCurve::NistP384),
        KeyType::Rsa(RsaBits::B3072),
    ] {
        let chave = KeyGen::new(tipo).comment("ana@art").generate().unwrap();
        let caminho = pasta.arquivo(tipo.file_name());
        chave.write(&caminho, true).unwrap();
        let publica = public_path(&caminho);
        let Some((ok, saida)) = rodar("ssh-keygen", &["-l", "-v", "-f", publica.to_str().unwrap()])
        else {
            return;
        };
        assert!(ok);
        let esperado = saida.trim_end();
        let nosso = format!(
            "{} {} {} ({})\n{}",
            kind_and_bits(chave.public_key()).1,
            chave.fingerprint(),
            chave.comment(),
            kind_and_bits(chave.public_key()).0,
            chave.randomart().trim_end()
        );
        assert_eq!(nosso, esperado, "{}", tipo.label());
    }
}

#[test]
fn o_openssh_le_a_chave_gerada() {
    let pasta = Pasta::nova("interop");
    let caminho = pasta.arquivo("id");
    let chave = KeyGen::new(KeyType::Ed25519)
        .comment("ana@interop")
        .passphrase("frase interop")
        .kdf(KdfLevel::High)
        .generate()
        .unwrap();
    chave.write(&caminho, false).unwrap();
    let Some((ok, linha)) = rodar(
        "ssh-keygen",
        &["-y", "-P", "frase interop", "-f", caminho.to_str().unwrap()],
    ) else {
        return;
    };
    assert!(ok, "o ssh-keygen não leu a chave");
    assert_eq!(linha.trim(), chave.public_openssh());
    let (ok, impressao) = rodar("ssh-keygen", &["-l", "-f", caminho.to_str().unwrap()]).unwrap();
    assert!(ok);
    assert!(impressao.contains(&chave.fingerprint()));
    // E a senha errada, não.
    let (ok, _) = rodar(
        "ssh-keygen",
        &["-y", "-P", "errada", "-f", caminho.to_str().unwrap()],
    )
    .unwrap();
    assert!(!ok);
}

#[test]
fn a_chave_trocada_pelo_rewrite_o_openssh_le() {
    let pasta = Pasta::nova("interop-rewrite");
    let caminho = pasta.arquivo("id");
    let chave = KeyGen::new(KeyType::Ed25519).generate().unwrap();
    chave.write(&caminho, false).unwrap();
    KeyFile::open(&caminho, None)
        .unwrap()
        .rewrite(&caminho, Some("nova"), KdfLevel::VeryHigh)
        .unwrap();
    let Some((ok, linha)) = rodar(
        "ssh-keygen",
        &["-y", "-P", "nova", "-f", caminho.to_str().unwrap()],
    ) else {
        return;
    };
    assert!(ok);
    assert_eq!(
        linha.trim().split(' ').nth(1),
        chave.public_openssh().split(' ').nth(1)
    );
}

#[test]
fn o_comentario_padrao_e_usuario_arroba_maquina() {
    let comentario = default_comment();
    if std::env::var("USER").is_ok() && std::env::var("HOSTNAME").is_ok() {
        assert!(comentario.contains('@'));
    }
    let chave = KeyGen::new(KeyType::Ed25519).generate().unwrap();
    assert_eq!(chave.comment(), comentario);
}

#[test]
fn um_arquivo_que_nao_e_chave_da_erro_claro() {
    let pasta = Pasta::nova("lixo");
    let caminho = pasta.arquivo("texto");
    fs::write(&caminho, "isto não é uma chave").unwrap();
    let erro = KeyFile::inspect(&caminho).unwrap_err();
    assert!(matches!(erro, KeyError::Invalid(_)));
    assert!(erro.to_string().starts_with("A chave não pôde ser lida"));
    assert!(matches!(
        KeyFile::inspect(&pasta.arquivo("nao-existe")),
        Err(KeyError::Io(_))
    ));
}
