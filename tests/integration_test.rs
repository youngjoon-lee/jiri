use dockertest::{Composition, DockerTest};

#[test]
fn integration_test() {
    let mut test = DockerTest::new();
    let jiri1 = Composition::with_repository("jiri-debug").with_container_name("jiri1");
    let jiri2 = Composition::with_repository("jiri-debug").with_container_name("jiri2");
    test.add_composition(jiri1);
    test.add_composition(jiri2);

    test.run(|ops| async move {
        let jiri1 = ops.handle("jiri1");
        println!("jiri1: {jiri1:?}");
        let jiri2 = ops.handle("jiri2");
        println!("jiri2: {jiri2:?}");
    });
}
