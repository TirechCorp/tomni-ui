//! A identidade contra o `zerotier-idtool` de verdade: as fixtures de `tests/fixtures/identidades`
//! saíram de `testes/zerotier/gerar-fixtures.sh`.

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use tomni_ui_zerotier::Identity;

fn fixture(nome: &str) -> String {
    let caminho = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/identidades")
        .join(nome);
    fs::read_to_string(&caminho)
        .unwrap_or_else(|e| panic!("{}: {e}", caminho.display()))
        .trim()
        .to_string()
}

fn secreta(n: u32) -> Identity {
    Identity::from_secret_str(&fixture(&format!("id{n}.secret"))).unwrap()
}

#[test]
fn aceita_as_identidades_geradas_pelo_zerotier() {
    for n in 1..=3 {
        let id = secreta(n);
        assert!(id.has_secret());
        assert!(id.validate(), "id{n} deve passar na prova de trabalho");
        assert_eq!(id.to_public_string(), fixture(&format!("id{n}.public")));
        assert_eq!(
            id.to_secret_string().unwrap().as_str(),
            fixture(&format!("id{n}.secret"))
        );
        let publica = Identity::from_public_str(&fixture(&format!("id{n}.public"))).unwrap();
        assert!(!publica.has_secret());
        assert!(publica.validate());
        assert_eq!(publica.address(), id.address());
    }
}

#[test]
fn byte_trocado_na_chave_ou_no_endereco_e_recusado() {
    let original = fixture("id1.public");
    let (endereco, resto) = original.split_once(':').unwrap();

    // Um dígito da chave pública (a parte X25519).
    let mut chave = original.clone().into_bytes();
    let posicao = endereco.len() + 3;
    chave[posicao] = if chave[posicao] == b'0' { b'1' } else { b'0' };
    let adulterada = Identity::from_public_str(std::str::from_utf8(&chave).unwrap()).unwrap();
    assert!(!adulterada.validate());

    // Outro endereço com a mesma chave.
    let outro = format!(
        "{}{}:{resto}",
        &endereco[..9],
        if endereco.ends_with('0') { '1' } else { '0' }
    );
    let outro = Identity::from_public_str(&outro).unwrap();
    assert!(!outro.validate());
}

#[test]
fn textos_malformados_sao_recusados() {
    let boa = fixture("id1.public");
    assert!(Identity::from_public_str("").is_err());
    assert!(Identity::from_public_str("ffffffffff:0:00").is_err());
    assert!(Identity::from_public_str(&boa.replacen(":0:", ":1:", 1)).is_err());
    assert!(Identity::from_public_str(&boa[..boa.len() - 2]).is_err());
    assert!(Identity::from_public_str(&format!("{boa}:00")).is_err());
}

#[test]
fn a_assinatura_do_zerotier_confere_e_a_nossa_e_igual_a_dele() {
    let mensagem = fixture("mensagem.txt") + "\n";
    for n in 1..=3 {
        let id = secreta(n);
        let do_zerotier = fixture(&format!("id{n}.assinatura"));
        let bytes: Vec<u8> = (0..do_zerotier.len() / 2)
            .map(|i| u8::from_str_radix(&do_zerotier[i * 2..i * 2 + 2], 16).unwrap())
            .collect();
        assert_eq!(bytes.len(), 96);
        assert!(
            id.verify(mensagem.as_bytes(), &bytes),
            "id{n}: a do ZeroTier"
        );

        // Ed25519 é determinístico: assinando a mesma mensagem sai a mesma assinatura.
        assert_eq!(
            id.sign(mensagem.as_bytes()).unwrap().as_slice(),
            &bytes[..],
            "id{n}"
        );

        let mut trocada = bytes.clone();
        trocada[10] ^= 1;
        assert!(!id.verify(mensagem.as_bytes(), &trocada));
        assert!(!id.verify(b"outra mensagem", &bytes));
        assert!(!secreta(if n == 3 { 1 } else { n + 1 }).verify(mensagem.as_bytes(), &bytes));
    }
}

#[test]
fn o_acordo_de_chaves_e_igual_nos_dois_lados() {
    let (a, b) = (secreta(1), secreta(2));
    let da = a.agree(&b).unwrap();
    let db = b.agree(&a).unwrap();
    assert_eq!(*da, *db);
    let publica = Identity::from_public_str(&fixture("id2.public")).unwrap();
    assert_eq!(*a.agree(&publica).unwrap(), *da);
    assert!(
        publica.agree(&a).is_none(),
        "sem a chave secreta não há acordo"
    );
}

#[test]
fn gera_uma_identidade_que_passa_na_validacao() {
    let inicio = Instant::now();
    let id = Identity::generate().unwrap();
    eprintln!("identidade gerada em {:?}", inicio.elapsed());
    assert!(id.validate());
    assert!(!id.address().is_reserved());
    let de_volta = Identity::from_secret_str(&id.to_secret_string().unwrap()).unwrap();
    assert_eq!(de_volta.address(), id.address());
    assert!(de_volta.validate());
}
