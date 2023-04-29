use dockertest::{Composition, DockerTest};

#[test]
fn integration_test() {
    let mut test = DockerTest::new();
    let jiri1 = Composition::with_repository("jiri-debug").with_container_name("jiri1");
    let jiri2 = Composition::with_repository("jiri-debug").with_container_name("jiri2");
    test.add_composition(jiri1);
    test.add_composition(jiri2);

    // let has_ran: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
    // let has_ran_test = has_ran.clone();

    test.run(|ops| async move {
        let jiri1 = ops.handle("jiri1");
        println!("jiri1: {jiri1:?}");
        let jiri2 = ops.handle("jiri2");
        println!("jiri2: {jiri2:?}");

        // The container is in a running state at this point.
        // Depending on the Image, it may exit on its own (like this hello-world image)
        // let mut ran = has_ran_test.lock().unwrap();
        // *ran = true;
    });

    // let ran = has_ran.lock().unwrap();
    // assert!(*ran);
}
