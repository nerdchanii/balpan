#[cfg(test)]
mod django_case_test {
    use indoc::indoc;
    use crate::integration_test::assert_analyzed_source_code;

    #[test]
    fn test_class_definition_within_class() {
        let source_code = indoc! {"
        class Car(models.Model):
            name = models.CharField(max_length=20)
            default_parts = models.ManyToManyField(Part)
            optional_parts = models.ManyToManyField(Part, related_name=\"cars_optional\")

            class Meta:
                ordering = (\"name\",)"};

        let result = indoc! {"
        # [TODO]
        class Car(models.Model):
            name = models.CharField(max_length=20)
            default_parts = models.ManyToManyField(Part)
            optional_parts = models.ManyToManyField(Part, related_name=\"cars_optional\")

            # [TODO]
            class Meta:
                ordering = (\"name\",)"};

        assert_analyzed_source_code(source_code, result, "python")
    }


    #[test]
    fn test_decorated_definitions_within_class_definition() {
        let source_code = indoc! {"      
        class Choices(enum.Enum, metaclass=ChoicesMeta):
            \"\"\"Class for creating enumerated choices.\"\"\"

            @DynamicClassAttribute
            def label(self):
                return self._label_

            @property
            def do_not_call_in_templates(self):
                return True"};

        let result = indoc! {"      
        # [TODO]
        class Choices(enum.Enum, metaclass=ChoicesMeta):
            \"\"\"Class for creating enumerated choices.\"\"\"

            # [TODO]
            @DynamicClassAttribute
            def label(self):
                return self._label_

            # [TODO]
            @property
            def do_not_call_in_templates(self):
                return True"};

        assert_analyzed_source_code(source_code, result, "python")
    }
}
