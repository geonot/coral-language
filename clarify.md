Ok, lets clarify some things about the language and the design. 

We want complete type inference, so we don't want to introduce even optional type hints or type declarations in methods, etc. We shouldn't need generics/templates because of the type inference. ALl of this stuff should just work, with an intelligent compiler. 

Regarding Result and Option types... we want these to be invisible/transparent, without having to unwrap, so it won't be a container, but more like an attribute of the Value.  We want to have all Value types be able to be a result / option, so that if for example a function returns some value, we can check to see if it is error or not on the actual value vs. a Result container. Consider how to represent this cleanly and clearly syntactically and how to achieve it on the backend. 

Regarding errors and error propagation, we want to have an err syntax but avoid the mess with go around checking each return value for error, and handling that explicitly. Syntactically, maybe something like the following, but lets think about the cleanest way to represent this that fits in line with the above regarding result and option - 

If we need enums implement them like the err is implemented. I changed the syntax from !! to err just now in the authoritative syntax.coral. 

*do_something(p)
    p.is_active() ? p.process() ! err NotActive


*some_function(bar)
    foo = do_something(bar) ? ! return err

    #can we shortcut like below instead? 
    #lets add this if it doesn't conflict
    foo = do_something(bar) ! return err
    

*go_ahead(x)
    return match something_else(x)
        101 ? 'ok'
        102 ? 'no'
            !  err #can just be naked err

*ga_error_handler(x)
    log('error: {x.err}')

*sf_error_handler()
    log('error something happened')

*main()
    x = some_function(input())
    x ? go_ahead(x) ?
        ! ga_error_handler()
      ! sf_error_handler(x)


We need to figure out what do do about libc, can we replace with coral version? If so, create a document about that with comprehensive task breakdown in the docs folder. Same with making system calls, assembly shims. For example, allocation at runtime, share a story
where this is fully handled by coral, same for low level client/server code, also show higher level standard library implementations / patterns for the same.

From LLVM we can go to a native binary (ELF), can we also go to wasm? What other compilation targets does this unlock? 

Create some code files that showcase Coral to this point. After all things have been considered, breakdown the Alpha roadmap into discrete tasks and sub tasks, and start working through the list, all the way to completion. For the pipeline operator, you can use ~, as it is unused and I don't want to use 2 characters. We will also for now omit or infer side-effect tracking. We do want a mixin/trait system, make it coral-like and straightforward. 

Ensure the items in the technical debt list are fully included in the todo list. as well as phases 4, 5 and 6. 